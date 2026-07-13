use std::collections::BTreeSet;
use std::fs;

use anyhow::{Context, Result};
use clap::Args;
use colored::Colorize;
use serde_json::Value;

use crate::{parse_content, Format};

#[derive(Args, Debug)]
pub(crate) struct DiffCli {
    pub(crate) left_file: String,
    pub(crate) right_file: String,

    #[arg(long, value_enum)]
    pub(crate) format: Option<Format>,

    #[arg(long)]
    pub(crate) show_secrets: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum DiffChange {
    Added { path: String, value: Value },
    Removed { path: String, value: Value },
    Changed { path: String, old: Value, new: Value },
}

enum DiffInput {
    Structured(Value, Value),
    Text(String, String),
}

pub(crate) fn run(cli: DiffCli, use_color: bool) -> Result<bool> {
    let input = load_diff_input(&cli)?;
    let output = match input {
        DiffInput::Structured(left, right) => {
            let changes = diff_values(&left, &right);
            if changes.is_empty() {
                println!("No differences");
                return Ok(false);
            }
            format_structural_diff(&changes, cli.show_secrets, use_color)
        }
        DiffInput::Text(left, right) => {
            if left == right {
                println!("No differences");
                return Ok(false);
            }
            format_text_diff(&left, &right, use_color)
        }
    };

    println!("{output}");
    Ok(true)
}

fn load_diff_input(cli: &DiffCli) -> Result<DiffInput> {
    let left = fs::read_to_string(&cli.left_file)
        .with_context(|| format!("Failed to read file: {}", cli.left_file))?;
    let right = fs::read_to_string(&cli.right_file)
        .with_context(|| format!("Failed to read file: {}", cli.right_file))?;

    if cli.format.is_some() {
        let left_value = parse_content(&cli.left_file, &left, cli.format)?;
        let right_value = parse_content(&cli.right_file, &right, cli.format)?;
        return Ok(DiffInput::Structured(left_value, right_value));
    }

    match (
        parse_content(&cli.left_file, &left, None),
        parse_content(&cli.right_file, &right, None),
    ) {
        (Ok(left_value), Ok(right_value)) => Ok(DiffInput::Structured(left_value, right_value)),
        _ => Ok(DiffInput::Text(left, right)),
    }
}

pub(crate) fn diff_values(left: &Value, right: &Value) -> Vec<DiffChange> {
    let mut changes = Vec::new();
    diff_value_at("", left, right, &mut changes);
    changes
}

fn diff_value_at(path: &str, left: &Value, right: &Value, changes: &mut Vec<DiffChange>) {
    match (left, right) {
        (Value::Object(left_map), Value::Object(right_map)) => {
            let keys: BTreeSet<&String> = left_map.keys().chain(right_map.keys()).collect();
            for key in keys {
                let child_path = object_path(path, key);
                match (left_map.get(key), right_map.get(key)) {
                    (Some(left_value), Some(right_value)) => {
                        diff_value_at(&child_path, left_value, right_value, changes);
                    }
                    (None, Some(value)) => changes.push(DiffChange::Added {
                        path: child_path,
                        value: value.clone(),
                    }),
                    (Some(value), None) => changes.push(DiffChange::Removed {
                        path: child_path,
                        value: value.clone(),
                    }),
                    (None, None) => {}
                }
            }
        }
        (Value::Array(left_items), Value::Array(right_items)) => {
            let max_len = left_items.len().max(right_items.len());
            for index in 0..max_len {
                let child_path = array_path(path, index);
                match (left_items.get(index), right_items.get(index)) {
                    (Some(left_value), Some(right_value)) => {
                        diff_value_at(&child_path, left_value, right_value, changes);
                    }
                    (None, Some(value)) => changes.push(DiffChange::Added {
                        path: child_path,
                        value: value.clone(),
                    }),
                    (Some(value), None) => changes.push(DiffChange::Removed {
                        path: child_path,
                        value: value.clone(),
                    }),
                    (None, None) => {}
                }
            }
        }
        _ if left == right => {}
        _ => changes.push(DiffChange::Changed {
            path: display_path(path),
            old: left.clone(),
            new: right.clone(),
        }),
    }
}

fn object_path(parent: &str, key: &str) -> String {
    if parent.is_empty() {
        key.to_string()
    } else {
        format!("{parent}.{key}")
    }
}

fn array_path(parent: &str, index: usize) -> String {
    if parent.is_empty() {
        format!("[{index}]")
    } else {
        format!("{parent}[{index}]")
    }
}

fn display_path(path: &str) -> String {
    if path.is_empty() {
        "(root)".to_string()
    } else {
        path.to_string()
    }
}

pub(crate) fn format_structural_diff(
    changes: &[DiffChange],
    show_secrets: bool,
    use_color: bool,
) -> String {
    let mut sections = Vec::new();

    let changed: Vec<&DiffChange> = changes
        .iter()
        .filter(|change| matches!(change, DiffChange::Changed { .. }))
        .collect();
    if !changed.is_empty() {
        let mut lines = vec![heading("Changed", use_color)];
        for change in changed {
            if let DiffChange::Changed { path, old, new } = change {
                lines.push(format!("{} {}", marker("~", use_color), path));
                if is_sensitive_path(path) && !show_secrets {
                    lines.push(format!("  {} <secret changed>", marker("-", use_color)));
                    lines.push(format!("  {} <secret changed>", marker("+", use_color)));
                } else {
                    lines.push(format!(
                        "  {} {}",
                        marker("-", use_color),
                        format_diff_value(old)
                    ));
                    lines.push(format!(
                        "  {} {}",
                        marker("+", use_color),
                        format_diff_value(new)
                    ));
                }
                lines.push(String::new());
            }
        }
        trim_trailing_blank(&mut lines);
        sections.push(lines.join("\n"));
    }

    let added: Vec<&DiffChange> = changes
        .iter()
        .filter(|change| matches!(change, DiffChange::Added { .. }))
        .collect();
    if !added.is_empty() {
        let mut lines = vec![heading("Added", use_color)];
        for change in added {
            if let DiffChange::Added { path, value } = change {
                let rendered = if is_sensitive_path(path) && !show_secrets {
                    "<secret>".to_string()
                } else {
                    format_diff_value(value)
                };
                lines.push(format!("{} {} = {}", marker("+", use_color), path, rendered));
            }
        }
        sections.push(lines.join("\n"));
    }

    let removed: Vec<&DiffChange> = changes
        .iter()
        .filter(|change| matches!(change, DiffChange::Removed { .. }))
        .collect();
    if !removed.is_empty() {
        let mut lines = vec![heading("Removed", use_color)];
        for change in removed {
            if let DiffChange::Removed { path, value } = change {
                let rendered = if is_sensitive_path(path) && !show_secrets {
                    "<secret>".to_string()
                } else {
                    format_diff_value(value)
                };
                lines.push(format!("{} {} = {}", marker("-", use_color), path, rendered));
            }
        }
        sections.push(lines.join("\n"));
    }

    sections.join("\n\n")
}

fn trim_trailing_blank(lines: &mut Vec<String>) {
    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
}

fn format_diff_value(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        _ => serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()),
    }
}

pub(crate) fn is_sensitive_path(path: &str) -> bool {
    let upper = path.to_ascii_uppercase();
    ["PASS", "PWD", "SECRET", "TOKEN", "KEY", "HASH", "CREDENTIAL"]
        .iter()
        .any(|needle| upper.contains(needle))
}

pub(crate) fn format_text_diff(left: &str, right: &str, use_color: bool) -> String {
    let left_lines: Vec<&str> = left.lines().collect();
    let right_lines: Vec<&str> = right.lines().collect();
    let pairs = lcs_pairs(&left_lines, &right_lines);
    let mut output = vec![heading("Text diff", use_color)];
    let mut left_index = 0;
    let mut right_index = 0;

    for (next_left, next_right) in pairs {
        while left_index < next_left {
            output.push(format!("{} {}", marker("-", use_color), left_lines[left_index]));
            left_index += 1;
        }
        while right_index < next_right {
            output.push(format!("{} {}", marker("+", use_color), right_lines[right_index]));
            right_index += 1;
        }
        output.push(format!("  {}", left_lines[next_left]));
        left_index = next_left + 1;
        right_index = next_right + 1;
    }

    while left_index < left_lines.len() {
        output.push(format!("{} {}", marker("-", use_color), left_lines[left_index]));
        left_index += 1;
    }
    while right_index < right_lines.len() {
        output.push(format!("{} {}", marker("+", use_color), right_lines[right_index]));
        right_index += 1;
    }

    output.join("\n")
}

fn lcs_pairs(left: &[&str], right: &[&str]) -> Vec<(usize, usize)> {
    let mut lengths = vec![vec![0usize; right.len() + 1]; left.len() + 1];
    for i in (0..left.len()).rev() {
        for j in (0..right.len()).rev() {
            lengths[i][j] = if left[i] == right[j] {
                lengths[i + 1][j + 1] + 1
            } else {
                lengths[i + 1][j].max(lengths[i][j + 1])
            };
        }
    }

    let mut pairs = Vec::new();
    let mut i = 0;
    let mut j = 0;
    while i < left.len() && j < right.len() {
        if left[i] == right[j] {
            pairs.push((i, j));
            i += 1;
            j += 1;
        } else if lengths[i + 1][j] >= lengths[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }

    pairs
}

fn heading(text: &str, use_color: bool) -> String {
    if use_color {
        text.bold().to_string()
    } else {
        text.to_string()
    }
}

fn marker(text: &str, use_color: bool) -> String {
    if !use_color {
        return text.to_string();
    }

    match text {
        "+" => text.green().bold().to_string(),
        "-" => text.red().bold().to_string(),
        "~" => text.yellow().bold().to_string(),
        _ => text.to_string(),
    }
}
