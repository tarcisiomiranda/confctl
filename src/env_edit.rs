//! In-place editing of config files: `confctl set` and `confctl unset`.
//!
//! For .env files: line-based editing that preserves comments, blank lines,
//! ordering, `export ` prefixes, and inline ` # comments`.
//! For JSON/YAML files: parse → mutate → serialize back (key order preserved).

use std::path::Path;

use anyhow::{bail, Context, Result};
use clap::Args;
use colored::Colorize;

#[derive(Args, Debug)]
pub(crate) struct SetCli {
    /// Path to the config file (.env, .json, .yaml, .yml). Created if it does not exist.
    pub(crate) file: String,
    /// One or more KEY=VALUE pairs to add or update. Dotted paths supported for JSON/YAML (e.g. db.port=5432).
    #[arg(required = true)]
    pub(crate) pairs: Vec<String>,
}

#[derive(Args, Debug)]
pub(crate) struct UnsetCli {
    /// Path to the config file (.env, .json, .yaml, .yml).
    pub(crate) file: String,
    /// One or more keys to remove. Dotted paths supported for JSON/YAML (e.g. db.port).
    #[arg(required = true)]
    pub(crate) keys: Vec<String>,
}

#[derive(Debug, PartialEq)]
pub(crate) enum SetOutcome {
    Added,
    Updated,
}

// ── Format detection ─────────────────────────────────────────────────────────

enum EditFormat {
    Env,
    Json,
    Yaml,
    Toml,
}

fn detect_edit_format(path: &Path) -> Result<EditFormat> {
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if file_name == ".env" || file_name.starts_with(".env.") {
        return Ok(EditFormat::Env);
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase());
    match ext.as_deref() {
        Some("json") => Ok(EditFormat::Json),
        Some("yaml" | "yml") => Ok(EditFormat::Yaml),
        Some("toml") => Ok(EditFormat::Toml),
        Some("env") => Ok(EditFormat::Env),
        Some(other) => bail!("unsupported file extension: .{other}"),
        None => Ok(EditFormat::Env),
    }
}

// ── ENV helpers ───────────────────────────────────────────────────────────────

/// The key on a non-comment line, tolerating an `export ` prefix.
fn line_key(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let rest = trimmed.strip_prefix("export ").unwrap_or(trimmed);
    let key = rest.split('=').next()?.trim();
    (!key.is_empty()).then_some(key)
}

/// Quote the value when it would not survive a round-trip unquoted.
fn render_value(value: &str) -> String {
    let needs_quotes = value.is_empty()
        || value.contains(char::is_whitespace)
        || value.contains('#')
        || value.contains('"');
    if needs_quotes {
        format!("\"{}\"", value.replace('"', "\\\""))
    } else {
        value.to_string()
    }
}

/// Rebuild an existing line with a new value, preserving indentation, an
/// `export ` prefix, and a trailing ` # comment`.
fn replace_value(line: &str, key: &str, value: &str) -> String {
    let indent_len = line.len() - line.trim_start().len();
    let indent = &line[..indent_len];
    let trimmed = &line[indent_len..];
    let export = if trimmed.starts_with("export ") {
        "export "
    } else {
        ""
    };

    let after_eq = line.split_once('=').map(|(_, v)| v).unwrap_or("");
    let comment = after_eq
        .char_indices()
        .find(|&(i, c)| c == '#' && after_eq[..i].ends_with(|p: char| p.is_whitespace()))
        .map(|(i, _)| format!("  #{}", &after_eq[i + 1..]))
        .unwrap_or_default();

    format!("{indent}{export}{key}={}{comment}", render_value(value))
}

/// Add or update `key` in ENV `content`.
pub(crate) fn set_key(content: &str, key: &str, value: &str) -> (String, SetOutcome) {
    let mut lines: Vec<String> = content.lines().map(str::to_string).collect();
    let mut updated = false;

    for line in lines.iter_mut() {
        if line_key(line) == Some(key) {
            *line = replace_value(line, key, value);
            updated = true;
        }
    }

    if !updated {
        lines.push(format!("{key}={}", render_value(value)));
    }

    let outcome = if updated {
        SetOutcome::Updated
    } else {
        SetOutcome::Added
    };
    (lines.join("\n") + "\n", outcome)
}

/// Remove every line assigning `key` from ENV `content`.
pub(crate) fn unset_key(content: &str, key: &str) -> (String, bool) {
    let mut removed = false;
    let kept: Vec<&str> = content
        .lines()
        .filter(|line| {
            let is_target = line_key(line) == Some(key);
            removed |= is_target;
            !is_target
        })
        .collect();

    let body = kept.join("\n");
    let output = if body.is_empty() { body } else { body + "\n" };
    (output, removed)
}

// ── JSON helpers ──────────────────────────────────────────────────────────────

fn coerce_json_value(existing: &serde_json::Value, new_str: &str) -> serde_json::Value {
    use serde_json::Value;
    match existing {
        Value::Bool(_) => match new_str.to_ascii_lowercase().as_str() {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            _ => Value::String(new_str.to_string()),
        },
        Value::Number(_) => {
            if let Ok(n) = new_str.parse::<i64>() {
                Value::Number(n.into())
            } else if let Ok(f) = new_str.parse::<f64>() {
                Value::Number(serde_json::Number::from_f64(f).unwrap_or_else(|| 0.into()))
            } else {
                Value::String(new_str.to_string())
            }
        }
        Value::Null if new_str == "null" => Value::Null,
        _ => Value::String(new_str.to_string()),
    }
}

fn json_set_path(
    root: &mut serde_json::Value,
    dotted_key: &str,
    new_str: &str,
) -> Result<SetOutcome> {
    use serde_json::{Map, Value};

    let segments: Vec<&str> = dotted_key.split('.').collect();
    let (last, parents) = segments.split_last().unwrap();
    let mut current = root;

    for segment in parents {
        match current {
            Value::Object(map) => {
                current = map
                    .entry(segment.to_string())
                    .or_insert_with(|| Value::Object(Map::new()));
            }
            _ => bail!("cannot traverse into non-object at '{segment}'"),
        }
    }

    match current {
        Value::Object(map) => {
            let new_val = map
                .get(*last)
                .map(|e| coerce_json_value(e, new_str))
                .unwrap_or_else(|| Value::String(new_str.to_string()));
            let existed = map.contains_key(*last);
            map.insert(last.to_string(), new_val);
            Ok(if existed {
                SetOutcome::Updated
            } else {
                SetOutcome::Added
            })
        }
        _ => bail!("cannot set key '{last}' on non-object"),
    }
}

fn json_unset_path(root: &mut serde_json::Value, dotted_key: &str) -> Result<bool> {
    use serde_json::Value;

    let segments: Vec<&str> = dotted_key.split('.').collect();
    let (last, parents) = segments.split_last().unwrap();
    let mut current = root;

    for segment in parents {
        match current {
            Value::Object(map) => match map.get_mut(*segment) {
                Some(val) => current = val,
                None => return Ok(false),
            },
            _ => return Ok(false),
        }
    }

    match current {
        Value::Object(map) => Ok(map.remove(*last).is_some()),
        _ => Ok(false),
    }
}

fn run_set_json(path: &Path, pairs: &[(&str, &str)]) -> Result<()> {
    let content = if path.exists() {
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?
    } else {
        "{}".to_string()
    };

    let mut root: serde_json::Value = serde_json::from_str(&content)
        .with_context(|| format!("parsing JSON: {}", path.display()))?;

    let mut report = Vec::new();
    for (key, value) in pairs {
        let outcome = json_set_path(&mut root, key, value)?;
        report.push((key.to_string(), outcome));
    }

    let new_content = serde_json::to_string_pretty(&root).context("serializing JSON")?;
    std::fs::write(path, new_content + "\n")
        .with_context(|| format!("writing {}", path.display()))?;

    print_report(report);
    Ok(())
}

fn run_unset_json(path: &Path, keys: &[&str]) -> Result<()> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;

    let mut root: serde_json::Value = serde_json::from_str(&content)
        .with_context(|| format!("parsing JSON: {}", path.display()))?;

    let mut report = Vec::new();
    for key in keys {
        let removed = json_unset_path(&mut root, key)?;
        report.push((key.to_string(), removed));
    }

    let new_content = serde_json::to_string_pretty(&root).context("serializing JSON")?;
    std::fs::write(path, new_content + "\n")
        .with_context(|| format!("writing {}", path.display()))?;

    print_unset_report(report);
    Ok(())
}

// ── YAML helpers ──────────────────────────────────────────────────────────────

fn coerce_yaml_value(existing: &serde_yaml::Value, new_str: &str) -> serde_yaml::Value {
    use serde_yaml::Value;
    match existing {
        Value::Bool(_) => match new_str.to_ascii_lowercase().as_str() {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            _ => Value::String(new_str.to_string()),
        },
        Value::Number(_) => {
            if let Ok(n) = new_str.parse::<i64>() {
                Value::Number(serde_yaml::Number::from(n))
            } else if let Ok(f) = new_str.parse::<f64>() {
                Value::Number(serde_yaml::Number::from(f))
            } else {
                Value::String(new_str.to_string())
            }
        }
        Value::Null if new_str == "null" || new_str == "~" => Value::Null,
        _ => Value::String(new_str.to_string()),
    }
}

fn yaml_set_path(
    root: &mut serde_yaml::Value,
    dotted_key: &str,
    new_str: &str,
) -> Result<SetOutcome> {
    use serde_yaml::Value;

    let segments: Vec<&str> = dotted_key.split('.').collect();
    let (last, parents) = segments.split_last().unwrap();
    let mut current = root;

    for segment in parents {
        match current {
            Value::Mapping(map) => {
                current = map
                    .entry(Value::String(segment.to_string()))
                    .or_insert(Value::Mapping(serde_yaml::Mapping::new()));
            }
            _ => bail!("cannot traverse into non-mapping at '{segment}'"),
        }
    }

    let k = serde_yaml::Value::String(last.to_string());
    match current {
        Value::Mapping(map) => {
            let new_val = map
                .get(&k)
                .map(|e| coerce_yaml_value(e, new_str))
                .unwrap_or_else(|| Value::String(new_str.to_string()));
            let existed = map.contains_key(&k);
            map.insert(k, new_val);
            Ok(if existed {
                SetOutcome::Updated
            } else {
                SetOutcome::Added
            })
        }
        _ => bail!("cannot set key '{last}' on non-mapping"),
    }
}

fn yaml_unset_path(root: &mut serde_yaml::Value, dotted_key: &str) -> Result<bool> {
    use serde_yaml::Value;

    let segments: Vec<&str> = dotted_key.split('.').collect();
    let (last, parents) = segments.split_last().unwrap();
    let mut current = root;

    for segment in parents {
        match current {
            Value::Mapping(map) => {
                let k = Value::String(segment.to_string());
                match map.get_mut(&k) {
                    Some(val) => current = val,
                    None => return Ok(false),
                }
            }
            _ => return Ok(false),
        }
    }

    let k = serde_yaml::Value::String(last.to_string());
    match current {
        Value::Mapping(map) => Ok(map.remove(&k).is_some()),
        _ => Ok(false),
    }
}

fn run_set_yaml(path: &Path, pairs: &[(&str, &str)]) -> Result<()> {
    let content = if path.exists() {
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?
    } else {
        String::new()
    };

    let mut root: serde_yaml::Value = serde_yaml::from_str(&content)
        .with_context(|| format!("parsing YAML: {}", path.display()))?;

    let mut report = Vec::new();
    for (key, value) in pairs {
        let outcome = yaml_set_path(&mut root, key, value)?;
        report.push((key.to_string(), outcome));
    }

    let new_content = serde_yaml::to_string(&root).context("serializing YAML")?;
    std::fs::write(path, &new_content).with_context(|| format!("writing {}", path.display()))?;

    print_report(report);
    Ok(())
}

fn run_unset_yaml(path: &Path, keys: &[&str]) -> Result<()> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;

    let mut root: serde_yaml::Value = serde_yaml::from_str(&content)
        .with_context(|| format!("parsing YAML: {}", path.display()))?;

    let mut report = Vec::new();
    for key in keys {
        let removed = yaml_unset_path(&mut root, key)?;
        report.push((key.to_string(), removed));
    }

    let new_content = serde_yaml::to_string(&root).context("serializing YAML")?;
    std::fs::write(path, &new_content).with_context(|| format!("writing {}", path.display()))?;

    print_unset_report(report);
    Ok(())
}

// ── TOML helpers ─────────────────────────────────────────────────────────────

fn coerce_toml_item(existing: &toml_edit::Item, new_str: &str) -> toml_edit::Item {
    match existing.as_value() {
        Some(toml_edit::Value::Integer(_)) => {
            if let Ok(n) = new_str.parse::<i64>() {
                toml_edit::value(n)
            } else {
                toml_edit::value(new_str)
            }
        }
        Some(toml_edit::Value::Float(_)) => {
            if let Ok(f) = new_str.parse::<f64>() {
                toml_edit::value(f)
            } else {
                toml_edit::value(new_str)
            }
        }
        Some(toml_edit::Value::Boolean(_)) => match new_str.to_ascii_lowercase().as_str() {
            "true" => toml_edit::value(true),
            "false" => toml_edit::value(false),
            _ => toml_edit::value(new_str),
        },
        _ => toml_edit::value(new_str),
    }
}

fn toml_set_path(
    doc: &mut toml_edit::DocumentMut,
    dotted_key: &str,
    new_str: &str,
) -> Result<SetOutcome> {
    let segments: Vec<&str> = dotted_key.split('.').collect();
    let (last, parents) = segments.split_last().unwrap();

    let mut current = doc.as_table_mut();
    for segment in parents {
        let item = current
            .entry(segment)
            .or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
        current = match item.as_table_mut() {
            Some(t) => t,
            None => bail!("'{segment}' is not a TOML table"),
        };
    }

    let new_item = current
        .get(last)
        .map(|e| coerce_toml_item(e, new_str))
        .unwrap_or_else(|| toml_edit::value(new_str));
    let existed = current.contains_key(last);
    current.insert(last, new_item);

    Ok(if existed {
        SetOutcome::Updated
    } else {
        SetOutcome::Added
    })
}

fn toml_unset_path(doc: &mut toml_edit::DocumentMut, dotted_key: &str) -> Result<bool> {
    let segments: Vec<&str> = dotted_key.split('.').collect();
    let (last, parents) = segments.split_last().unwrap();

    let mut current = doc.as_table_mut();
    for segment in parents {
        current = match current.get_mut(segment) {
            Some(item) => match item.as_table_mut() {
                Some(t) => t,
                None => bail!("'{segment}' is not a TOML table"),
            },
            None => return Ok(false),
        };
    }

    Ok(current.remove(last).is_some())
}

fn run_set_toml(path: &Path, pairs: &[(&str, &str)]) -> Result<()> {
    let content = if path.exists() {
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?
    } else {
        String::new()
    };

    let mut doc: toml_edit::DocumentMut = content
        .parse()
        .with_context(|| format!("parsing TOML: {}", path.display()))?;

    let mut report = Vec::new();
    for (key, value) in pairs {
        let outcome = toml_set_path(&mut doc, key, value)?;
        report.push((key.to_string(), outcome));
    }

    std::fs::write(path, doc.to_string()).with_context(|| format!("writing {}", path.display()))?;

    print_report(report);
    Ok(())
}

fn run_unset_toml(path: &Path, keys: &[&str]) -> Result<()> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;

    let mut doc: toml_edit::DocumentMut = content
        .parse()
        .with_context(|| format!("parsing TOML: {}", path.display()))?;

    let mut report = Vec::new();
    for key in keys {
        let removed = toml_unset_path(&mut doc, key)?;
        report.push((key.to_string(), removed));
    }

    std::fs::write(path, doc.to_string()).with_context(|| format!("writing {}", path.display()))?;

    print_unset_report(report);
    Ok(())
}

// ── Shared output ─────────────────────────────────────────────────────────────

fn print_report(report: Vec<(String, SetOutcome)>) {
    for (key, outcome) in report {
        let verb = match outcome {
            SetOutcome::Added => "added",
            SetOutcome::Updated => "updated",
        };
        println!("{} {verb} {}", "✓".green().bold(), key.bold());
    }
}

fn print_unset_report(report: Vec<(String, bool)>) {
    for (key, removed) in report {
        if removed {
            println!("{} removed {}", "✓".green().bold(), key.bold());
        } else {
            println!(
                "{} {} not found (nothing to remove)",
                "·".bright_black(),
                key
            );
        }
    }
}

// ── Key/pair parsing ──────────────────────────────────────────────────────────

fn parse_pair(pair: &str) -> Result<(&str, &str)> {
    let (key, value) = pair
        .split_once('=')
        .with_context(|| format!("expected KEY=VALUE, got {pair:?}"))?;
    let key = key.trim();
    if key.is_empty() {
        bail!("invalid key: must not be empty");
    }
    Ok((key, value))
}

fn parse_env_key(key: &str) -> Result<&str> {
    let key = key.trim();
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        bail!("invalid key {key:?}: use letters, digits, and `_`");
    }
    Ok(key)
}

// ── Public entry points ───────────────────────────────────────────────────────

pub(crate) fn run_set(cli: SetCli) -> Result<()> {
    let path = Path::new(&cli.file);
    match detect_edit_format(path)? {
        EditFormat::Json => {
            let pairs: Result<Vec<_>> = cli.pairs.iter().map(|p| parse_pair(p)).collect();
            run_set_json(path, &pairs?)
        }
        EditFormat::Yaml => {
            let pairs: Result<Vec<_>> = cli.pairs.iter().map(|p| parse_pair(p)).collect();
            run_set_yaml(path, &pairs?)
        }
        EditFormat::Toml => {
            let pairs: Result<Vec<_>> = cli.pairs.iter().map(|p| parse_pair(p)).collect();
            run_set_toml(path, &pairs?)
        }
        EditFormat::Env => {
            let mut content = if path.exists() {
                std::fs::read_to_string(path)
                    .with_context(|| format!("reading {}", path.display()))?
            } else {
                String::new()
            };

            let mut report = Vec::new();
            for pair in &cli.pairs {
                let (key, value) = parse_pair(pair)?;
                let key = parse_env_key(key)?;
                let (next, outcome) = set_key(&content, key, value);
                content = next;
                report.push((key.to_string(), outcome));
            }

            std::fs::write(path, &content)
                .with_context(|| format!("writing {}", path.display()))?;
            print_report(report);
            Ok(())
        }
    }
}

pub(crate) fn run_unset(cli: UnsetCli) -> Result<()> {
    let path = Path::new(&cli.file);
    match detect_edit_format(path)? {
        EditFormat::Json => {
            let keys: Vec<&str> = cli.keys.iter().map(String::as_str).collect();
            run_unset_json(path, &keys)
        }
        EditFormat::Yaml => {
            let keys: Vec<&str> = cli.keys.iter().map(String::as_str).collect();
            run_unset_yaml(path, &keys)
        }
        EditFormat::Toml => {
            let keys: Vec<&str> = cli.keys.iter().map(String::as_str).collect();
            run_unset_toml(path, &keys)
        }
        EditFormat::Env => {
            let mut content = std::fs::read_to_string(path)
                .with_context(|| format!("reading {}", path.display()))?;

            let mut report = Vec::new();
            for key in &cli.keys {
                let (next, removed) = unset_key(&content, key);
                content = next;
                report.push((key.clone(), removed));
            }

            std::fs::write(path, &content)
                .with_context(|| format!("writing {}", path.display()))?;
            print_unset_report(report);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = "\
# Database configuration
DB_HOST=localhost
DB_PORT=5432  # default postgres port

export API_URL=https://example.com
DEBUG=true
";

    #[test]
    fn set_updates_existing_key_in_place() {
        let (out, outcome) = set_key(FIXTURE, "DB_HOST", "10.0.0.5");
        assert_eq!(outcome, SetOutcome::Updated);
        assert!(out.contains("DB_HOST=10.0.0.5"));
        assert!(!out.contains("localhost"));
        assert!(out.contains("# Database configuration"));
        assert!(out.contains("DEBUG=true"));
    }

    #[test]
    fn set_preserves_inline_comment_and_export_prefix() {
        let (out, _) = set_key(FIXTURE, "DB_PORT", "6543");
        assert!(out.contains("DB_PORT=6543  # default postgres port"));

        let (out, _) = set_key(FIXTURE, "API_URL", "https://new.example.com");
        assert!(out.contains("export API_URL=https://new.example.com"));
    }

    #[test]
    fn set_appends_missing_key_at_end() {
        let (out, outcome) = set_key(FIXTURE, "NEW_KEY", "value");
        assert_eq!(outcome, SetOutcome::Added);
        assert!(out.ends_with("NEW_KEY=value\n"));
        assert!(out.starts_with("# Database configuration\n"));
    }

    #[test]
    fn set_quotes_values_that_need_it() {
        let (out, _) = set_key(FIXTURE, "GREETING", "hello world");
        assert!(out.contains("GREETING=\"hello world\""));

        let (out, _) = set_key(FIXTURE, "EMPTY", "");
        assert!(out.contains("EMPTY=\"\""));

        let (out, _) = set_key(FIXTURE, "PLAIN", "no-quotes-needed");
        assert!(out.contains("PLAIN=no-quotes-needed"));
    }

    #[test]
    fn unset_removes_key_but_keeps_comments() {
        let (out, removed) = unset_key(FIXTURE, "DB_HOST");
        assert!(removed);
        assert!(!out.contains("DB_HOST"));
        assert!(out.contains("# Database configuration"));
        assert!(out.contains("DB_PORT=5432"));
    }

    #[test]
    fn unset_missing_key_is_a_noop() {
        let (out, removed) = unset_key(FIXTURE, "NOPE");
        assert!(!removed);
        assert_eq!(out, FIXTURE);
    }

    #[test]
    fn unset_handles_export_prefix() {
        let (out, removed) = unset_key(FIXTURE, "API_URL");
        assert!(removed);
        assert!(!out.contains("API_URL"));
    }

    #[test]
    fn key_must_not_match_comments_or_substrings() {
        let content = "# DB_HOST=old\nDB_HOST_EXTRA=1\n";
        let (out, removed) = unset_key(content, "DB_HOST");
        assert!(!removed);
        assert_eq!(out, content);
    }

    #[test]
    fn parse_pair_validates_key_charset() {
        assert!(parse_pair("GOOD_KEY=v").is_ok());
        assert!(parse_pair("no-equals").is_err());
        assert!(parse_pair("=v").is_err());
    }

    // JSON tests

    #[test]
    fn json_set_updates_existing_string_field() {
        let json = r#"{"host": "localhost", "port": 5432}"#;
        let mut root: serde_json::Value = serde_json::from_str(json).unwrap();
        let outcome = json_set_path(&mut root, "host", "10.0.0.5").unwrap();
        assert_eq!(outcome, SetOutcome::Updated);
        assert_eq!(root["host"], "10.0.0.5");
        assert_eq!(root["port"], 5432); // untouched
    }

    #[test]
    fn json_set_preserves_number_type() {
        let json = r#"{"port": 5432}"#;
        let mut root: serde_json::Value = serde_json::from_str(json).unwrap();
        json_set_path(&mut root, "port", "9090").unwrap();
        assert_eq!(root["port"], 9090i64);
    }

    #[test]
    fn json_set_preserves_bool_type() {
        let json = r#"{"debug": false}"#;
        let mut root: serde_json::Value = serde_json::from_str(json).unwrap();
        json_set_path(&mut root, "debug", "true").unwrap();
        assert_eq!(root["debug"], true);
    }

    #[test]
    fn json_set_adds_missing_key_as_string() {
        let json = r#"{"host": "localhost"}"#;
        let mut root: serde_json::Value = serde_json::from_str(json).unwrap();
        let outcome = json_set_path(&mut root, "api_key", "sk-secret").unwrap();
        assert_eq!(outcome, SetOutcome::Added);
        assert_eq!(root["api_key"], "sk-secret");
    }

    #[test]
    fn json_set_dotted_path() {
        let json = r#"{"db": {"host": "localhost", "port": 5432}}"#;
        let mut root: serde_json::Value = serde_json::from_str(json).unwrap();
        json_set_path(&mut root, "db.host", "10.0.0.5").unwrap();
        assert_eq!(root["db"]["host"], "10.0.0.5");
        assert_eq!(root["db"]["port"], 5432);
    }

    #[test]
    fn json_unset_removes_key() {
        let json = r#"{"host": "localhost", "port": 5432}"#;
        let mut root: serde_json::Value = serde_json::from_str(json).unwrap();
        let removed = json_unset_path(&mut root, "host").unwrap();
        assert!(removed);
        assert!(root.as_object().unwrap().get("host").is_none());
        assert_eq!(root["port"], 5432);
    }

    #[test]
    fn json_unset_missing_key_returns_false() {
        let json = r#"{"host": "localhost"}"#;
        let mut root: serde_json::Value = serde_json::from_str(json).unwrap();
        let removed = json_unset_path(&mut root, "nope").unwrap();
        assert!(!removed);
    }
}
