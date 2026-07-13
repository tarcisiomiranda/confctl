//! Non-secret config file for the `vault` subcommand.
//!
//! Lookup order for **reads** (first existing wins):
//!   1. `$CONFCTL_CONFIG` — explicit override, useful in CI / containers.
//!   2. `~/.config/confctl/vault.toml` — per-user, written by `vault login`.
//!   3. `/etc/confctl/vault.toml`      — system-wide default, read-only.
//!
//! Writes always target `~/.config/confctl/vault.toml` (unless the user
//! passes `--config PATH`). This keeps `/etc/confctl/` as read-only system
//! config that doesn't need sudo to update per-user sessions.
//!
//! Each backend nests under its own key (`[bunker]`, `[hcp]`, ...). The
//! top-level `backend = "…"` picks which one is active by default; the
//! `--backend` CLI flag overrides.
//!
//! This file may reference credential files (`token_file`, `credentials_file`,
//! etc.) on disk — confctl never writes secret material to `/etc/`. Secrets
//! earned at login time live in the OS keyring (with a file fallback under
//! the user's config dir).

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendKind {
    Bunker,
    Hcp,
    Aws,
    Gcp,
    Azure,
}

impl BackendKind {
    pub const ALL: [BackendKind; 5] = [
        BackendKind::Bunker,
        BackendKind::Hcp,
        BackendKind::Aws,
        BackendKind::Gcp,
        BackendKind::Azure,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            BackendKind::Bunker => "bunker",
            BackendKind::Hcp => "hcp",
            BackendKind::Aws => "aws",
            BackendKind::Gcp => "gcp",
            BackendKind::Azure => "azure",
        }
    }
}

impl fmt::Display for BackendKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for BackendKind {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "bunker" => Ok(BackendKind::Bunker),
            "hcp" | "hashicorp" | "vault" => Ok(BackendKind::Hcp),
            "aws" => Ok(BackendKind::Aws),
            "gcp" | "google" => Ok(BackendKind::Gcp),
            "azure" | "az" => Ok(BackendKind::Azure),
            other => Err(format!(
                "unknown backend `{other}`; expected one of: bunker, hcp, aws, gcp, azure"
            )),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VaultConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<BackendKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bunker: Option<BunkerConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hcp: Option<HcpConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gcp: Option<GcpConfig>,
    // Other backends' sections will land here in follow-up PRs:
    //   pub aws: Option<AwsConfig>,
    //   pub azure: Option<AzureConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BunkerConfig {
    pub url: String,
    pub email: String,
    pub kdf_salt_b64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HcpConfig {
    /// Vault base URL — e.g. `https://vault.example.com:8200`. Matches the
    /// upstream `VAULT_ADDR` convention.
    pub addr: String,
    /// KV v2 mount path. Defaults to `secret`.
    #[serde(default = "default_hcp_mount")]
    pub mount: String,
    /// Enterprise namespace, sent as `X-Vault-Namespace`. Omitted when None.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    /// Path to a file holding the Vault token. Read at call time, never
    /// written to. Typical placement: `/etc/confctl/hcp-token` chmod 0600.
    /// Takes precedence over `VAULT_TOKEN` env var but loses to a token
    /// cached by `vault login`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_file: Option<PathBuf>,
}

fn default_hcp_mount() -> String {
    "secret".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GcpConfig {
    /// GCP project ID that hosts Secret Manager.
    pub project: String,
    /// Optional explicit credentials JSON (authorized_user ADC shape).
    /// When absent, auth falls back to `GOOGLE_APPLICATION_CREDENTIALS`,
    /// then the gcloud ADC file, then the `gcloud` CLI, then the GCE
    /// metadata server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credentials_file: Option<PathBuf>,
}

/// System-wide default config path, read-only.
pub const SYSTEM_CONFIG_PATH: &str = "/etc/confctl/vault.toml";

/// Env var that overrides every lookup — must be an absolute path.
pub const CONFIG_ENV_VAR: &str = "CONFCTL_CONFIG";

impl VaultConfig {
    /// Where `login` writes. Always the per-user config dir.
    pub fn default_write_path() -> Result<PathBuf> {
        let dir = dirs::config_dir().context("could not resolve user config dir")?;
        Ok(dir.join("confctl").join("vault.toml"))
    }

    /// Where reads look: `$CONFCTL_CONFIG` → `~/.config/confctl/vault.toml`
    /// → `/etc/confctl/vault.toml`. The first existing path wins. If none
    /// exist we fall back to the user path so error messages still point at
    /// the place a later `vault login` will write to.
    pub fn resolve_read_path() -> Result<PathBuf> {
        if let Ok(p) = std::env::var(CONFIG_ENV_VAR) {
            if !p.is_empty() {
                return Ok(PathBuf::from(p));
            }
        }
        let user = Self::default_write_path()?;
        if user.exists() {
            return Ok(user);
        }
        let system = PathBuf::from(SYSTEM_CONFIG_PATH);
        if system.exists() {
            return Ok(system);
        }
        Ok(user)
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("reading vault config from {}", path.display()))?;
        let cfg: VaultConfig = toml::from_str(&content)
            .with_context(|| format!("parsing vault config at {}", path.display()))?;
        Ok(cfg)
    }

    /// Like `load_from`, but returns `Default::default()` when the file is
    /// missing. Used by backends that need to read partial config without
    /// asserting a prior login.
    pub fn load_or_default(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        Self::load_from(path)
    }

    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating config dir {}", parent.display()))?;
        }
        let body = toml::to_string_pretty(self).context("serializing vault config")?;
        std::fs::write(path, body)
            .with_context(|| format!("writing vault config to {}", path.display()))?;
        set_user_only_mode(path)?;
        Ok(())
    }
}

#[cfg(unix)]
fn set_user_only_mode(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o600);
    std::fs::set_permissions(path, perms)
        .with_context(|| format!("chmod 0600 {}", path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_user_only_mode(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn tempfile() -> PathBuf {
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("confctl-vault-config-{pid}-{nanos}.toml"))
    }

    #[test]
    fn roundtrip_bunker() {
        let path = tempfile();
        let cfg = VaultConfig {
            backend: Some(BackendKind::Bunker),
            bunker: Some(BunkerConfig {
                url: "https://vault.example.com".into(),
                email: "me@example.com".into(),
                kdf_salt_b64: "AAECAwQFBgcICQoLDA0ODw==".into(),
            }),
            ..Default::default()
        };
        cfg.save_to(&path).unwrap();

        let loaded = VaultConfig::load_from(&path).unwrap();
        assert_eq!(loaded.backend, Some(BackendKind::Bunker));
        let b = loaded.bunker.unwrap();
        assert_eq!(b.url, "https://vault.example.com");
        assert_eq!(b.email, "me@example.com");

        let mut body = String::new();
        std::fs::File::open(&path)
            .unwrap()
            .read_to_string(&mut body)
            .unwrap();
        assert!(body.contains("backend = \"bunker\""));
        assert!(body.contains("[bunker]"));

        std::fs::remove_file(&path).ok();
    }

    #[cfg(unix)]
    #[test]
    fn save_sets_0600() {
        use std::os::unix::fs::PermissionsExt;
        let path = tempfile();
        VaultConfig::default().save_to(&path).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "file should be 0600, got {mode:o}");

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn kind_parsing() {
        assert_eq!(
            BackendKind::from_str("bunker").unwrap(),
            BackendKind::Bunker
        );
        assert_eq!(BackendKind::from_str("HCP").unwrap(), BackendKind::Hcp);
        assert_eq!(
            BackendKind::from_str("vault").unwrap(),
            BackendKind::Hcp,
            "`vault` is accepted as an alias for HashiCorp Vault"
        );
        assert_eq!(
            BackendKind::from_str("google").unwrap(),
            BackendKind::Gcp,
            "`google` is accepted as an alias for GCP"
        );
        assert!(BackendKind::from_str("foo").is_err());
    }

    #[test]
    fn load_or_default_tolerates_missing() {
        let path = tempfile();
        assert!(!path.exists());
        let cfg = VaultConfig::load_or_default(&path).unwrap();
        assert!(cfg.backend.is_none());
        assert!(cfg.bunker.is_none());
    }
}
