use std::fs;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};
use serde_json::Value;

#[derive(Parser)]
#[command(
    name = "confctl",
    version,
    about = "A simplified jq for configuration files (JSON, YAML, TOML)"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Get { file: String, path: String },
    Print { file: String },
}

enum Format {
    Json,
    Yaml,
    Toml,
}

fn detect_format(file_path: &str) -> Result<Format> {
    let ext = Path::new(file_path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase());

    match ext.as_deref() {
        Some("json") => Ok(Format::Json),
        Some("yaml" | "yml") => Ok(Format::Yaml),
        Some("toml") => Ok(Format::Toml),
        Some(other) => {
            bail!("Unsupported file extension: .{other}. Supported: .json, .yaml, .yml, .toml")
        }
        None => bail!("Could not determine file format: no extension found on '{file_path}'"),
    }
}

fn parse_file(file_path: &str) -> Result<Value> {
    let content = fs::read_to_string(file_path)
        .with_context(|| format!("Failed to read file: {file_path}"))?;

    let format = detect_format(file_path)?;

    let value = match format {
        Format::Json => serde_json::from_str::<Value>(&content)
            .with_context(|| format!("Failed to parse JSON: {file_path}"))?,
        Format::Yaml => serde_yaml::from_str::<Value>(&content)
            .with_context(|| format!("Failed to parse YAML: {file_path}"))?,
        Format::Toml => {
            let toml_value: toml::Value = toml::from_str(&content)
                .with_context(|| format!("Failed to parse TOML: {file_path}"))?;
            let json_str =
                serde_json::to_string(&toml_value).context("Failed to serialize TOML to JSON")?;
            serde_json::from_str::<Value>(&json_str).context("Failed to deserialize TOML-JSON")?
        }
    };

    Ok(value)
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

fn format_value(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        _ => serde_json::to_string_pretty(value).unwrap_or_else(|_| format!("{value:?}")),
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Get { file, path } => {
            let value = parse_file(&file)?;
            let result = resolve_path(&value, &path)?;
            println!("{}", format_value(result));
        }
        Commands::Print { file } => {
            let value = parse_file(&file)?;
            let pretty = serde_json::to_string_pretty(&value)
                .context("Failed to serialize value to JSON")?;
            println!("{pretty}");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_resolve_simple_key() {
        let data = json!({"club": {"name": "Vasco da Gama", "founded": 1898}});
        let result = resolve_path(&data, "club.name").unwrap();
        assert_eq!(result, &json!("Vasco da Gama"));
    }

    #[test]
    fn test_resolve_numeric_index() {
        let data = json!({"players": [{"name": "Edmundo"}, {"name": "Juninho Pernambucano"}]});
        let result = resolve_path(&data, "players.1.name").unwrap();
        assert_eq!(result, &json!("Juninho Pernambucano"));
    }

    #[test]
    fn test_resolve_missing_key() {
        let data = json!({"club": {"name": "Vasco da Gama"}});
        let result = resolve_path(&data, "club.stadium");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Key not found"));
    }

    #[test]
    fn test_resolve_index_out_of_bounds() {
        let data = json!({"titles": [1, 2, 3]});
        let result = resolve_path(&data, "titles.5");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("out of bounds"));
    }

    #[test]
    fn test_resolve_scalar_traversal() {
        let data = json!({"name": "Vasco da Gama"});
        let result = resolve_path(&data, "name.something");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("scalar value"));
    }

    #[test]
    fn test_format_string_no_quotes() {
        assert_eq!(format_value(&json!("Edmundo")), "Edmundo");
    }

    #[test]
    fn test_format_number() {
        assert_eq!(format_value(&json!(9)), "9");
    }

    #[test]
    fn test_format_bool() {
        assert_eq!(format_value(&json!(true)), "true");
    }
}
