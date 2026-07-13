//! In-place editing of .env files: `confctl set` and `confctl unset`.
//!
//! Edits are line-based, never parse→reserialize, so comments, blank lines,
//! ordering, `export ` prefixes, and inline ` # comments` all survive. This
//! exists so tools (and AI agents) can mutate a .env without reading it.

use std::path::Path;

use anyhow::{bail, Context, Result};
use clap::Args;
use colored::Colorize;

#[derive(Args, Debug)]
pub(crate) struct SetCli {
    /// Path to the .env file. Created if it does not exist.
    pub(crate) file: String,
    /// One or more KEY=VALUE pairs to add or update.
    #[arg(required = true)]
    pub(crate) pairs: Vec<String>,
}

#[derive(Args, Debug)]
pub(crate) struct UnsetCli {
    /// Path to the .env file.
    pub(crate) file: String,
    /// One or more keys to remove.
    #[arg(required = true)]
    pub(crate) keys: Vec<String>,
}

#[derive(Debug, PartialEq)]
pub(crate) enum SetOutcome {
    Added,
    Updated,
}

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
/// `export ` prefix, and a trailing ` # comment` (dotenv convention: a `#`
/// preceded by whitespace starts a comment).
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

/// Add or update `key` in `content`. Every non-comment line holding the key
/// is rewritten (duplicates stay duplicates, all with the new value); a
/// missing key is appended at the end.
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

/// Remove every line assigning `key`. Comments mentioning the key are left
/// untouched. Returns whether anything was removed.
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

fn parse_pair(pair: &str) -> Result<(&str, &str)> {
    let (key, value) = pair
        .split_once('=')
        .with_context(|| format!("expected KEY=VALUE, got {pair:?}"))?;
    let key = key.trim();
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        bail!("invalid key {key:?}: use letters, digits, and `_`");
    }
    Ok((key, value))
}

pub(crate) fn run_set(cli: SetCli) -> Result<()> {
    let path = Path::new(&cli.file);
    let mut content = if path.exists() {
        std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?
    } else {
        String::new()
    };

    let mut report = Vec::new();
    for pair in &cli.pairs {
        let (key, value) = parse_pair(pair)?;
        let (next, outcome) = set_key(&content, key, value);
        content = next;
        report.push((key.to_string(), outcome));
    }

    std::fs::write(path, &content).with_context(|| format!("writing {}", path.display()))?;

    for (key, outcome) in report {
        let verb = match outcome {
            SetOutcome::Added => "added",
            SetOutcome::Updated => "updated",
        };
        println!("{} {verb} {}", "✓".green().bold(), key.bold());
    }
    Ok(())
}

pub(crate) fn run_unset(cli: UnsetCli) -> Result<()> {
    let path = Path::new(&cli.file);
    let mut content = std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;

    let mut report = Vec::new();
    for key in &cli.keys {
        let (next, removed) = unset_key(&content, key);
        content = next;
        report.push((key.clone(), removed));
    }

    std::fs::write(path, &content).with_context(|| format!("writing {}", path.display()))?;

    for (key, removed) in report {
        if removed {
            println!("{} removed {}", "✓".green().bold(), key.bold());
        } else {
            println!("{} {} not found (nothing to remove)", "·".bright_black(), key);
        }
    }
    Ok(())
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
        // Everything else untouched, including the comment lines.
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
        // "# DB_HOST=old" commented out must not count as the key.
        let content = "# DB_HOST=old\nDB_HOST_EXTRA=1\n";
        let (out, removed) = unset_key(content, "DB_HOST");
        assert!(!removed);
        assert_eq!(out, content);
    }

    #[test]
    fn parse_pair_validates_key_charset() {
        assert!(parse_pair("GOOD_KEY=v").is_ok());
        assert!(parse_pair("no-equals").is_err());
        assert!(parse_pair("bad key=v").is_err());
        assert!(parse_pair("=v").is_err());
    }
}
