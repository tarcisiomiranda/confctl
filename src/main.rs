use std::fs;
use std::io::{self, Read};
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use clap::{Parser, Subcommand, ValueEnum};
use colored::Colorize;
use serde_json::{Map, Value};

mod diff;
mod env_edit;
mod vault;

#[derive(Parser)]
#[command(
    name = "confctl",
    version,
    about = "CLI for querying configuration files (JSON, YAML, TOML, ENV)"
)]
struct Cli {
    file: Option<String>,
    path: Option<String>,

    #[arg(long, value_enum)]
    format: Option<Format>,

    #[arg(short = 'd', long = "decode", conflicts_with = "encode")]
    decode: bool,

    #[arg(short = 'e', long = "encode", conflicts_with = "decode")]
    encode: bool,

    /// Mask sensitive values before printing: keys matching PASS, PWD, SECRET, TOKEN, KEY,
    /// HASH, CREDENTIAL plus values with known secret shapes (ghp_*, sk-*, AKIA*, JWTs, PEM).
    #[arg(short = 'r', long = "redact")]
    redact: bool,

    /// Minified single-line JSON output — ideal for CI/pipeline env vars.
    #[arg(short = 'c', long = "compact")]
    compact: bool,

    /// Also copy the final output to the system clipboard
    /// (wl-copy / xclip / xsel / pbcopy, first one found).
    #[arg(long = "copy")]
    copy: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Compare two config files with a human-friendly diff.
    Diff(diff::DiffCli),

    /// Add or update KEY=VALUE entries in a .env file, preserving comments.
    Set(env_edit::SetCli),

    /// Remove keys from a .env file, preserving comments.
    Unset(env_edit::UnsetCli),

    /// Push/pull secret files to a Bunker Vault server.
    Vault(vault::cli::VaultCli),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum Format {
    Json,
    Yaml,
    Toml,
    Env,
}

fn looks_like_env_format(content: &str) -> bool {
    let mut valid_lines = 0;
    let mut total_non_empty = 0;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        total_non_empty += 1;

        if let Some(pos) = line.find('=') {
            let key = &line[..pos];
            if !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                valid_lines += 1;
            }
        }
    }

    total_non_empty > 0 && valid_lines == total_non_empty
}

fn detect_format(file_path: &str, content: &str, forced_format: Option<Format>) -> Result<Format> {
    if let Some(format) = forced_format {
        return Ok(format);
    }

    let path = Path::new(file_path);
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

    if file_name == ".env" || file_name.starts_with(".env.") {
        return Ok(Format::Env);
    }

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase());

    match ext.as_deref() {
        Some("json") => Ok(Format::Json),
        Some("yaml" | "yml") => Ok(Format::Yaml),
        Some("toml") => Ok(Format::Toml),
        Some("env") => Ok(Format::Env),
        Some(other) => {
            bail!(
                "Unsupported file extension: .{other}. Supported: .json, .yaml, .yml, .toml, .env"
            )
        }
        None => {
            if looks_like_env_format(content) {
                Ok(Format::Env)
            } else if serde_json::from_str::<Value>(content).is_ok() {
                Ok(Format::Json)
            } else if toml::from_str::<toml::Value>(content).is_ok() {
                Ok(Format::Toml)
            } else if serde_yaml::from_str::<Value>(content).is_ok() {
                Ok(Format::Yaml)
            } else {
                bail!(
                    "Could not determine file format for '{file_path}'. Use a known extension or pass --format."
                )
            }
        }
    }
}

fn parse_env_format(content: &str) -> Value {
    let mut map = Map::new();

    for line in content.lines() {
        let line = line.trim();

        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if let Some(pos) = line.find('=') {
            let key = line[..pos].trim().to_string();
            let mut value = line[pos + 1..].trim();

            if (value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\''))
            {
                value = &value[1..value.len() - 1];
            }

            let json_value = if let Ok(n) = value.parse::<i64>() {
                Value::Number(n.into())
            } else if let Ok(n) = value.parse::<f64>() {
                Value::Number(serde_json::Number::from_f64(n).unwrap_or_else(|| 0.into()))
            } else if value.eq_ignore_ascii_case("true") {
                Value::Bool(true)
            } else if value.eq_ignore_ascii_case("false") {
                Value::Bool(false)
            } else {
                Value::String(value.to_string())
            };

            map.insert(key, json_value);
        }
    }

    Value::Object(map)
}

pub(crate) fn parse_content(
    file_path: &str,
    content: &str,
    forced_format: Option<Format>,
) -> Result<Value> {
    let format = detect_format(file_path, content, forced_format)?;

    let value = match format {
        Format::Json => serde_json::from_str::<Value>(content)
            .with_context(|| format!("Failed to parse JSON: {file_path}"))?,
        Format::Yaml => serde_yaml::from_str::<Value>(content)
            .with_context(|| format!("Failed to parse YAML: {file_path}"))?,
        Format::Toml => {
            let toml_value: toml::Value = toml::from_str(content)
                .with_context(|| format!("Failed to parse TOML: {file_path}"))?;
            let json_str =
                serde_json::to_string(&toml_value).context("Failed to serialize TOML to JSON")?;
            serde_json::from_str::<Value>(&json_str).context("Failed to deserialize TOML-JSON")?
        }
        Format::Env => parse_env_format(content),
    };

    Ok(value)
}

fn parse_file(file_path: &str, forced_format: Option<Format>) -> Result<Value> {
    let content = if file_path == "-" {
        let mut input = String::new();
        io::stdin()
            .read_to_string(&mut input)
            .context("Failed to read from stdin")?;
        input
    } else {
        fs::read_to_string(file_path)
            .with_context(|| format!("Failed to read file: {file_path}"))?
    };

    parse_content(file_path, &content, forced_format)
}

fn resolve_path<'a>(value: &'a Value, dotted_path: &str) -> Result<&'a Value> {
    let segments: Vec<&str> = dotted_path.split('.').collect();
    let mut current = value;

    for (i, segment) in segments.iter().enumerate() {
        let path_so_far = segments[..=i].join(".");

        match current {
            Value::Object(map) => {
                current = map.get(*segment).ok_or_else(|| {
                    anyhow!("Key not found: '{segment}' (at path '{path_so_far}')")
                })?;
            }
            Value::Array(arr) => {
                let index: usize = segment.parse().map_err(|_| {
                    anyhow!(
                        "Expected numeric index for array access, got '{segment}' (at path '{path_so_far}')"
                    )
                })?;
                current = arr.get(index).ok_or_else(|| {
                    anyhow!(
                        "Array index {index} out of bounds (length {}) at path '{path_so_far}'",
                        arr.len()
                    )
                })?;
            }
            _ => {
                let parent_path = segments[..i].join(".");
                bail!(
                    "Cannot traverse into a scalar value at '{parent_path}' \
                     (trying to access '{segment}')"
                );
            }
        }
    }

    Ok(current)
}

fn format_value_with(value: &Value, compact: bool) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        _ if compact => serde_json::to_string(value).unwrap_or_else(|_| format!("{value:?}")),
        _ => serde_json::to_string_pretty(value).unwrap_or_else(|_| format!("{value:?}")),
    }
}

/// Pipe `text` into the first clipboard tool that works. The confirmation
/// goes to stderr so stdout stays clean for pipes.
fn copy_to_clipboard(text: &str) -> Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    const TOOLS: &[(&str, &[&str])] = &[
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["--clipboard", "--input"]),
        ("pbcopy", &[]),
    ];

    for (tool, args) in TOOLS {
        let spawned = Command::new(tool)
            .args(*args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        let Ok(mut child) = spawned else { continue };
        if let Some(mut stdin) = child.stdin.take() {
            if stdin.write_all(text.as_bytes()).is_err() {
                continue;
            }
        }
        match child.wait() {
            Ok(status) if status.success() => {
                eprintln!("{} copied to clipboard ({tool})", "✓".green().bold());
                return Ok(());
            }
            _ => continue,
        }
    }
    bail!("no clipboard tool available — install wl-clipboard (Wayland), xclip/xsel (X11), or pbcopy (macOS)")
}

fn colorize_json(value: &Value, indent: usize) -> String {
    let indent_str = "  ".repeat(indent);
    let next_indent = "  ".repeat(indent + 1);

    match value {
        Value::Null => "null".bright_black().bold().to_string(),
        Value::Bool(b) => b.to_string().white().to_string(),
        Value::Number(n) => n.to_string().white().to_string(),
        Value::String(s) => format!("\"{}\"", s).green().to_string(),
        Value::Array(arr) => {
            if arr.is_empty() {
                "[]".to_string()
            } else {
                let items: Vec<String> = arr
                    .iter()
                    .map(|v| format!("{}{}", next_indent, colorize_json(v, indent + 1)))
                    .collect();
                format!("[\n{}\n{}]", items.join(",\n"), indent_str)
            }
        }
        Value::Object(map) => {
            if map.is_empty() {
                "{}".to_string()
            } else {
                let items: Vec<String> = map
                    .iter()
                    .map(|(k, v)| {
                        format!(
                            "{}{}: {}",
                            next_indent,
                            format!("\"{}\"", k).blue().bold(),
                            colorize_json(v, indent + 1)
                        )
                    })
                    .collect();
                format!("{{\n{}\n{}}}", items.join(",\n"), indent_str)
            }
        }
    }
}

fn format_value_colored(value: &Value) -> String {
    match value {
        Value::String(s) => s.green().to_string(),
        Value::Null => "null".bright_black().bold().to_string(),
        Value::Bool(b) => b.to_string().white().to_string(),
        Value::Number(n) => n.to_string().white().to_string(),
        _ => colorize_json(value, 0),
    }
}

fn looks_like_secret_value(value: &str) -> bool {
    const SECRET_PREFIXES: &[&str] = &[
        "ghp_",
        "gho_",
        "ghu_",
        "ghs_",
        "ghr_",
        "github_pat_",
        "glpat-",
        "sk-",
        "sk_live_",
        "sk_test_",
        "rk_live_",
        "rk_test_",
        "xoxb-",
        "xoxp-",
        "xoxa-",
        "xoxs-",
        "xapp-",
        "npm_",
        "pypi-",
        "AKIA",
        "AIza",
        "-----BEGIN",
    ];

    if SECRET_PREFIXES.iter().any(|p| value.starts_with(p)) {
        return true;
    }

    // JWT: three dot-separated base64url segments, header always starts with "eyJ".
    value.starts_with("eyJ") && value.matches('.').count() == 2
}

fn redact_sensitive(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, val)| {
                    let new_val = if diff::is_sensitive_path(key) {
                        Value::String("<redacted>".to_string())
                    } else {
                        redact_sensitive(val)
                    };
                    (key.clone(), new_val)
                })
                .collect(),
        ),
        Value::Array(arr) => Value::Array(arr.iter().map(redact_sensitive).collect()),
        Value::String(s) if looks_like_secret_value(s) => {
            Value::String("<redacted>".to_string())
        }
        other => other.clone(),
    }
}

fn apply_base64_transform(input: &str, decode: bool, encode: bool) -> Result<String> {
    if decode {
        let decoded = STANDARD
            .decode(input.trim())
            .context("Failed to decode base64")?;
        String::from_utf8(decoded).context("Decoded base64 is not valid UTF-8")
    } else if encode {
        Ok(STANDARD.encode(input))
    } else {
        Ok(input.to_string())
    }
}

fn interactive_usage_tutorial() -> &'static str {
    "No input detected.

Mini tutorial:
  confctl config.yaml clubs.0.name
  confctl config.toml
  cat config.json | confctl user.name
  curl -s https://api.github.com/users | confctl
  curl -s https://api.github.com/users | confctl 0.login --format json

Tip: use '-' to force stdin explicitly:
  curl -s https://api.github.com/users | confctl - 0.login

Run 'confctl --help' for full usage."
}

fn resolve_input(
    file: Option<String>,
    path: Option<String>,
    stdin_is_tty: bool,
) -> Result<(String, Option<String>)> {
    match (file, path) {
        (Some(file), Some(path)) => Ok((file, Some(path))),
        (Some(file), None) => {
            if file == "-" {
                return Ok((file, None));
            }

            if !stdin_is_tty && !Path::new(&file).exists() {
                Ok(("-".to_string(), Some(file)))
            } else {
                Ok((file, None))
            }
        }
        (None, maybe_path) => {
            if stdin_is_tty {
                bail!(interactive_usage_tutorial());
            }
            Ok(("-".to_string(), maybe_path))
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let use_color = atty::is(atty::Stream::Stdout);

    if let Some(command) = cli.command {
        match command {
            Command::Diff(diff_cli) => {
                if diff::run(diff_cli, use_color)? {
                    std::process::exit(1);
                }
                return Ok(());
            }
            Command::Set(set_cli) => return env_edit::run_set(set_cli),
            Command::Unset(unset_cli) => return env_edit::run_unset(unset_cli),
            Command::Vault(vault_cli) => return vault::cli::run(vault_cli),
        }
    }

    let stdin_is_tty = atty::is(atty::Stream::Stdin);

    let (file, path) = resolve_input(cli.file, cli.path, stdin_is_tty)?;

    let mut value = parse_file(&file, cli.format)?;

    if cli.redact {
        value = redact_sensitive(&value);
    }

    let final_output = match path {
        Some(path) => {
            let result = resolve_path(&value, &path)?;
            let output = format_value_with(result, cli.compact);
            let final_output = apply_base64_transform(&output, cli.decode, cli.encode)?;

            if cli.decode || cli.encode {
                print!("{}", final_output);
            } else if use_color && !cli.compact {
                println!("{}", format_value_colored(result));
            } else {
                println!("{}", final_output);
            }
            final_output
        }
        None => {
            let json_str = if cli.compact {
                serde_json::to_string(&value).context("Failed to serialize value to JSON")?
            } else {
                serde_json::to_string_pretty(&value)
                    .context("Failed to serialize value to JSON")?
            };
            if cli.encode {
                let encoded = STANDARD.encode(&json_str);
                print!("{}", encoded);
                encoded
            } else {
                if use_color && !cli.compact {
                    println!("{}", colorize_json(&value, 0));
                } else {
                    println!("{json_str}");
                }
                json_str
            }
        }
    };

    if cli.copy {
        copy_to_clipboard(&final_output)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests;
