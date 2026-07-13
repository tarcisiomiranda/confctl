//! Backend trait + helpers — one implementation per remote secret store.
//!
//! The CLI in `vault::cli` only ever sees `Box<dyn Backend>`; every
//! provider-specific quirk (auth flow, on-the-wire shape, client-side crypto,
//! etc.) lives in its own module here.

use std::path::Path;

use anyhow::{bail, Result};
use chrono::{DateTime, Utc};

use super::config::{BackendKind, VaultConfig};

pub mod bunker;
pub mod gcp;
pub mod hcp;

/// What the CLI passes to `Backend::login`. Fields are optional — the
/// backend prompts / reads env vars for anything still missing.
#[derive(Debug, Default, Clone)]
pub struct LoginOpts {
    /// Endpoint URL (bunker, hcp) / region (aws) / vault URI (azure) / project (gcp).
    pub endpoint: Option<String>,
    /// Identity: email (bunker), namespace / role, …
    pub identity: Option<String>,
}

/// Backend-side view of a stored secret's metadata.
#[derive(Debug, Clone)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub size: Option<u64>,
    pub updated_at: Option<DateTime<Utc>>,
    pub labels: Option<Vec<String>>,
    pub filename: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PushRequest<'a> {
    pub name: String,
    pub bytes: &'a [u8],
    pub mime: Option<String>,
    pub labels: Vec<String>,
    pub overwrite: bool,
    pub filename: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PullRequest<'a> {
    pub name: Option<&'a str>,
    pub id: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct PulledSecret {
    pub bytes: Vec<u8>,
    pub filename: Option<String>,
}

/// Session state reported by `Backend::status`.
#[derive(Debug, Clone)]
pub enum SessionStatus {
    /// Logged in; `expires_at` is optional because some providers have no
    /// explicit TTL (e.g. ambient AWS/GCP credentials).
    Valid { expires_at: Option<DateTime<Utc>> },
    /// Logged in but the session has expired.
    Expired,
    /// Nothing cached — user needs to run `vault login`.
    Missing,
    /// Auth is ambient (env vars / instance role); no confctl-managed session.
    Ambient,
}

#[derive(Debug, Clone)]
pub struct BackendStatus {
    pub kind: &'static str,
    pub endpoint: String,
    pub identity: Option<String>,
    pub session: SessionStatus,
    pub master_key_cached: Option<bool>,
}

pub trait Backend {
    fn kind(&self) -> &'static str;

    fn login(&mut self, opts: LoginOpts) -> Result<()>;
    fn logout(&mut self) -> Result<()>;
    fn status(&self) -> Result<BackendStatus>;

    fn list(&mut self) -> Result<Vec<Entry>>;
    fn push(&mut self, req: PushRequest<'_>) -> Result<Entry>;
    fn pull(&mut self, req: PullRequest<'_>) -> Result<PulledSecret>;
    fn rm(&mut self, name: &str) -> Result<()>;
}

/// Pick the right backend for a CLI invocation.
///
/// `read_path` is where config is loaded from (may be `/etc/confctl/...`).
/// `write_path` is where `login` will persist any config changes (always the
/// per-user path unless the user passed `--config`).
/// `override_kind` comes from the `--backend` CLI flag; when set it wins over
/// what's in the config file.
pub fn open(
    read_path: &Path,
    write_path: &Path,
    override_kind: Option<BackendKind>,
) -> Result<Box<dyn Backend>> {
    let cfg = VaultConfig::load_or_default(read_path)?;
    let kind = override_kind.or(cfg.backend).ok_or_else(|| {
        anyhow::anyhow!(
            "no backend configured. Pass --backend <bunker|hcp|aws|gcp|azure> or run `confctl vault login` first."
        )
    })?;
    match kind {
        BackendKind::Bunker => Ok(Box::new(bunker::BunkerBackend::new(
            write_path.to_path_buf(),
            cfg,
        )?)),
        BackendKind::Hcp => Ok(Box::new(hcp::HcpBackend::new(
            write_path.to_path_buf(),
            cfg,
        )?)),
        BackendKind::Gcp => Ok(Box::new(gcp::GcpBackend::new(
            write_path.to_path_buf(),
            cfg,
        )?)),
        BackendKind::Aws => bail!("the `aws` backend is not implemented yet"),
        BackendKind::Azure => bail!("the `azure` backend is not implemented yet"),
    }
}
