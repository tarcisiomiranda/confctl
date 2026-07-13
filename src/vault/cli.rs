//! Thin clap dispatcher over `Box<dyn Backend>`. Provider-specific logic
//! lives in `vault::backends::{bunker, hcp, aws, gcp, azure}`.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use colored::Colorize;

use super::backends::{self, LoginOpts, PullRequest, PushRequest, SessionStatus};
use super::config::{BackendKind, VaultConfig};

#[derive(Args, Debug)]
pub struct VaultCli {
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    /// Select which backend to target for this invocation. Falls back to the
    /// `backend = "..."` entry in the config file. Accepts: bunker, hcp
    /// (alias: hashicorp, vault), aws, gcp (alias: google), azure.
    #[arg(long, global = true, value_parser = parse_backend_kind)]
    pub backend: Option<BackendKind>,

    #[command(subcommand)]
    pub cmd: VaultCommand,
}

fn parse_backend_kind(s: &str) -> std::result::Result<BackendKind, String> {
    s.parse()
}

#[derive(Subcommand, Debug)]
pub enum VaultCommand {
    /// Authenticate against the selected backend and cache the session.
    Login {
        /// Endpoint URL (bunker / hcp) or region/project/vault-uri for
        /// provider backends. Alias: --url.
        #[arg(long, alias = "url")]
        endpoint: Option<String>,
        /// Identity (bunker: email; hcp: namespace). Alias: --email.
        #[arg(long, alias = "email")]
        identity: Option<String>,
    },
    /// Invalidate stored credentials for the selected backend.
    Logout,
    /// Show active backend, endpoint, identity, and session TTL.
    Status,
    /// List secrets stored on the selected backend.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Push a file (or stdin via `-`) as a secret on the selected backend.
    Push {
        /// Path to the file, or `-` to read stdin.
        source: String,
        /// Display name used by list/pull/rm. Defaults to an automatic
        /// `{current-dir}-{filename}-{YYYY-MM-DD}` slug, e.g. `myapp-env-2026-07-12`.
        #[arg(long)]
        name: Option<String>,
        /// MIME type override. Defaults to application/octet-stream.
        #[arg(long)]
        mime: Option<String>,
        /// Optional tag(s). Backend-specific behaviour: some index labels,
        /// others store them as opaque metadata.
        #[arg(long)]
        labels: Vec<String>,
        /// Replace an existing secret with the same display name.
        #[arg(long)]
        overwrite: bool,
    },
    /// Pull a secret by display name (or --id) and write its contents.
    Pull {
        /// Display name of the secret. Mutually exclusive with --id.
        name: Option<String>,
        #[arg(long, conflicts_with = "name")]
        id: Option<String>,
        /// Output path. Omit to use the stored filename; `-` to write to stdout.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Overwrite an existing output file.
        #[arg(long)]
        force: bool,
    },
    /// Delete a secret by display name.
    Rm {
        name: String,
        #[arg(long)]
        yes: bool,
    },
}

/// Path to read from: `--config` flag > `$CONFCTL_CONFIG` > user > /etc/.
fn read_config_path(override_path: Option<&PathBuf>) -> Result<PathBuf> {
    if let Some(p) = override_path {
        return Ok(p.clone());
    }
    VaultConfig::resolve_read_path()
}

/// Path for `vault login` to write to: `--config` flag > `$CONFCTL_CONFIG`
/// > user config dir. Never `/etc/` — the system path is read-only.
fn write_config_path(override_path: Option<&PathBuf>) -> Result<PathBuf> {
    if let Some(p) = override_path {
        return Ok(p.clone());
    }
    if let Ok(p) = std::env::var(super::config::CONFIG_ENV_VAR) {
        if !p.is_empty() {
            return Ok(PathBuf::from(p));
        }
    }
    VaultConfig::default_write_path()
}

pub fn run(cli: VaultCli) -> Result<()> {
    let read = read_config_path(cli.config.as_ref())?;
    let write = write_config_path(cli.config.as_ref())?;
    match cli.cmd {
        VaultCommand::Login { endpoint, identity } => {
            cmd_login(&read, &write, cli.backend, endpoint, identity)
        }
        VaultCommand::Logout => cmd_logout(&read, &write, cli.backend),
        VaultCommand::Status => cmd_status(&read, &write, cli.backend),
        VaultCommand::List { json } => cmd_list(&read, &write, cli.backend, json),
        VaultCommand::Push {
            source,
            name,
            mime,
            labels,
            overwrite,
        } => cmd_push(
            &read,
            &write,
            cli.backend,
            source,
            name,
            mime,
            labels,
            overwrite,
        ),
        VaultCommand::Pull {
            name,
            id,
            out,
            force,
        } => cmd_pull(&read, &write, cli.backend, name, id, out, force),
        VaultCommand::Rm { name, yes } => cmd_rm(&read, &write, cli.backend, name, yes),
    }
}

fn cmd_login(
    read: &Path,
    write: &Path,
    backend: Option<BackendKind>,
    endpoint: Option<String>,
    identity: Option<String>,
) -> Result<()> {
    let mut backend_impl = backends::open(read, write, backend)?;
    backend_impl.login(LoginOpts { endpoint, identity })
}

fn cmd_logout(read: &Path, write: &Path, backend: Option<BackendKind>) -> Result<()> {
    let mut backend_impl = match backends::open(read, write, backend) {
        Ok(b) => b,
        Err(_) => {
            println!("{} no session to log out from.", "·".bright_black());
            return Ok(());
        }
    };
    backend_impl.logout()?;
    println!("{} logged out.", "✓".green().bold());
    Ok(())
}

fn cmd_status(read: &Path, write: &Path, backend: Option<BackendKind>) -> Result<()> {
    let backend_impl = match backends::open(read, write, backend) {
        Ok(b) => b,
        Err(_) => {
            println!(
                "{} not configured. Run `confctl vault login`.",
                "·".bright_black()
            );
            return Ok(());
        }
    };
    let status = backend_impl.status()?;

    println!("{} {}", "backend:".bold(), status.kind);
    if !status.endpoint.is_empty() {
        println!("{} {}", "endpoint:".bold(), status.endpoint);
    }
    if let Some(id) = &status.identity {
        println!("{} {}", "identity:".bold(), id);
    }
    match &status.session {
        SessionStatus::Valid { expires_at } => match expires_at {
            Some(ts) => println!("{} valid (expires {})", "session:".bold(), ts.to_rfc3339()),
            None => println!("{} valid", "session:".bold()),
        },
        SessionStatus::Expired => println!("{} {}", "session:".bold(), "expired".yellow()),
        SessionStatus::Missing => println!("{} missing", "session:".bold()),
        SessionStatus::Ambient => println!("{} ambient credentials", "session:".bold()),
    }
    if let Some(cached) = status.master_key_cached {
        println!(
            "{} {}",
            "master-key:".bold(),
            if cached { "present" } else { "missing" }
        );
    }
    Ok(())
}

fn cmd_list(read: &Path, write: &Path, backend: Option<BackendKind>, json_out: bool) -> Result<()> {
    let mut backend_impl = backends::open(read, write, backend)?;
    let items = backend_impl.list()?;

    if json_out {
        let entries: Vec<serde_json::Value> = items
            .iter()
            .map(|e| {
                serde_json::json!({
                    "id": e.id,
                    "name": e.name,
                    "size": e.size,
                    "updated_at": e.updated_at.map(|d| d.to_rfc3339()),
                    "labels": e.labels,
                    "filename": e.filename,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&entries)?);
        return Ok(());
    }

    if items.is_empty() {
        println!("{}", "(no secrets)".bright_black());
        return Ok(());
    }

    println!(
        "{:<36}  {:<24}  {:>10}  {}",
        "ID".bold(),
        "NAME".bold(),
        "SIZE".bold(),
        "UPDATED".bold()
    );
    for e in &items {
        let size = e
            .size
            .map(|n| n.to_string())
            .unwrap_or_else(|| "-".to_string());
        let updated = e
            .updated_at
            .map(|d| d.to_rfc3339())
            .unwrap_or_else(|| "-".to_string());
        println!("{:<36}  {:<24}  {:>10}  {}", e.id, e.name, size, updated);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn cmd_push(
    read: &Path,
    write: &Path,
    backend: Option<BackendKind>,
    source: String,
    name: Option<String>,
    mime: Option<String>,
    labels: Vec<String>,
    overwrite: bool,
) -> Result<()> {
    let (bytes, default_name) = read_source(&source)?;
    let display_name = name
        .clone()
        .unwrap_or_else(|| auto_secret_name(&default_name));

    let mut backend_impl = backends::open(read, write, backend)?;
    let entry = backend_impl.push(PushRequest {
        name: display_name.clone(),
        bytes: &bytes,
        mime,
        labels,
        overwrite,
        filename: Some(default_name),
    })?;

    println!(
        "{} {} {} ({} bytes)",
        "✓".green().bold(),
        if overwrite { "updated" } else { "pushed" },
        display_name.bold(),
        bytes.len()
    );
    if !entry.id.is_empty() {
        println!("  id: {}", entry.id);
    }
    Ok(())
}

fn cmd_pull(
    read: &Path,
    write: &Path,
    backend: Option<BackendKind>,
    name: Option<String>,
    id: Option<String>,
    out: Option<PathBuf>,
    force: bool,
) -> Result<()> {
    if name.is_none() && id.is_none() {
        bail!("provide a display name or --id");
    }

    let mut backend_impl = backends::open(read, write, backend)?;
    let pulled = backend_impl.pull(PullRequest {
        name: name.as_deref(),
        id: id.as_deref(),
    })?;

    match out {
        None => {
            let default_name = pulled
                .filename
                .clone()
                .or_else(|| name.clone())
                .ok_or_else(|| anyhow::anyhow!("cannot determine output filename"))?;
            let path = PathBuf::from(default_name);
            write_file(&path, &pulled.bytes, force)?;
            println!(
                "{} wrote {} bytes to {}",
                "✓".green().bold(),
                pulled.bytes.len(),
                path.display()
            );
        }
        Some(p) if p.as_os_str() == "-" => {
            io::stdout()
                .write_all(&pulled.bytes)
                .context("writing stdout")?;
        }
        Some(p) => {
            write_file(&p, &pulled.bytes, force)?;
            println!(
                "{} wrote {} bytes to {}",
                "✓".green().bold(),
                pulled.bytes.len(),
                p.display()
            );
        }
    }
    Ok(())
}

fn cmd_rm(
    read: &Path,
    write: &Path,
    backend: Option<BackendKind>,
    name: String,
    yes: bool,
) -> Result<()> {
    if !yes {
        let reply = prompt_line(&format!("Delete {name:?}? [y/N] "))?;
        if !reply.eq_ignore_ascii_case("y") {
            println!("{} cancelled", "·".bright_black());
            return Ok(());
        }
    }
    let mut backend_impl = backends::open(read, write, backend)?;
    backend_impl.rm(&name)?;
    println!("{} removed {}", "✓".green().bold(), name.bold());
    Ok(())
}

// ---------- helpers ----------

/// Default secret name when `--name` is omitted:
/// `{current-dir-basename}-{filename}-{YYYY-MM-DD}`, slugged to the
/// `[A-Za-z0-9_-]` charset every backend accepts (GCP is the strictest).
/// `.env` inside /srv/myapp on 2026-07-12 → `myapp-env-2026-07-12`.
pub(crate) fn auto_secret_name(filename: &str) -> String {
    let dir = std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_default();
    let date = chrono::Local::now().format("%Y-%m-%d");
    let raw = if dir.is_empty() {
        format!("{filename}-{date}")
    } else {
        format!("{dir}-{filename}-{date}")
    };
    super::backends::gcp::sanitize_secret_id(&raw)
}

fn read_source(source: &str) -> Result<(Vec<u8>, String)> {
    if source == "-" {
        let mut buf = Vec::new();
        io::stdin().read_to_end(&mut buf).context("reading stdin")?;
        return Ok((buf, "stdin".into()));
    }
    let path = Path::new(source);
    let bytes = std::fs::read(path).with_context(|| format!("reading file {}", path.display()))?;
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(source)
        .to_string();
    Ok((bytes, name))
}

fn write_file(path: &Path, bytes: &[u8], force: bool) -> Result<()> {
    if path.exists() && !force {
        bail!(
            "{} already exists; pass --force to overwrite",
            path.display()
        );
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
    }
    std::fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn prompt_line(prompt: &str) -> Result<String> {
    print!("{prompt}");
    io::stdout().flush().ok();
    let mut s = String::new();
    io::stdin()
        .read_line(&mut s)
        .context("reading from stdin")?;
    Ok(s.trim().to_string())
}
