//! SecretStore: keeps master_key + tokens out of the plaintext config file.
//!
//! Default path: OS keyring (Secret Service / Keychain / Credential Manager).
//! Fallback: `~/.config/confctl/vault-secrets.toml` (chmod 0600), used only
//! when the platform has no keyring backend available. The fallback writes
//! a stderr warning the first time it engages.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::crypto::KEY_LEN;

const KEYRING_SERVICE: &str = "confctl";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthTokens {
    pub access: String,
    pub refresh: String,
    pub expires_at: DateTime<Utc>,
}

pub trait SecretStore {
    fn put_master_key(&self, email: &str, key: &[u8; KEY_LEN]) -> Result<()>;
    fn get_master_key(&self, email: &str) -> Result<Option<[u8; KEY_LEN]>>;
    fn put_tokens(&self, email: &str, tokens: &AuthTokens) -> Result<()>;
    fn get_tokens(&self, email: &str) -> Result<Option<AuthTokens>>;
    fn clear(&self, email: &str) -> Result<()>;
}

pub fn default_store() -> Box<dyn SecretStore> {
    match KeyringStore::probe() {
        Ok(store) => Box::new(store),
        Err(err) => {
            let fallback = FileStore::default_path().expect("resolving fallback secret store path");
            eprintln!(
                "warning: OS keyring unavailable ({err}); falling back to {} (chmod 0600)",
                fallback.display()
            );
            Box::new(FileStore::new(fallback))
        }
    }
}

// ---------- KeyringStore ----------

pub struct KeyringStore;

impl KeyringStore {
    pub fn probe() -> Result<Self> {
        let probe = keyring::Entry::new(KEYRING_SERVICE, "__probe__")
            .context("creating keyring probe entry")?;
        match probe.get_password() {
            Ok(_) => Ok(Self),
            Err(keyring::Error::NoEntry) => Ok(Self),
            Err(e) => Err(anyhow::anyhow!(e)),
        }
    }

    fn entry(email: &str, slot: &str) -> Result<keyring::Entry> {
        let user = format!("{email}:{slot}");
        keyring::Entry::new(KEYRING_SERVICE, &user).context("creating keyring entry")
    }
}

impl SecretStore for KeyringStore {
    fn put_master_key(&self, email: &str, key: &[u8; KEY_LEN]) -> Result<()> {
        let encoded = STANDARD.encode(key);
        Self::entry(email, "master-key")?
            .set_password(&encoded)
            .context("writing master-key to keyring")
    }

    fn get_master_key(&self, email: &str) -> Result<Option<[u8; KEY_LEN]>> {
        match Self::entry(email, "master-key")?.get_password() {
            Ok(s) => {
                let bytes = STANDARD
                    .decode(s)
                    .context("decoding master-key from keyring")?;
                if bytes.len() != KEY_LEN {
                    anyhow::bail!("master-key wrong length in keyring");
                }
                let mut out = [0u8; KEY_LEN];
                out.copy_from_slice(&bytes);
                Ok(Some(out))
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(anyhow::anyhow!(e)),
        }
    }

    fn put_tokens(&self, email: &str, tokens: &AuthTokens) -> Result<()> {
        let body = serde_json::to_string(tokens).context("serializing tokens")?;
        Self::entry(email, "tokens")?
            .set_password(&body)
            .context("writing tokens to keyring")
    }

    fn get_tokens(&self, email: &str) -> Result<Option<AuthTokens>> {
        match Self::entry(email, "tokens")?.get_password() {
            Ok(s) => {
                let t: AuthTokens =
                    serde_json::from_str(&s).context("parsing tokens from keyring")?;
                Ok(Some(t))
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(anyhow::anyhow!(e)),
        }
    }

    fn clear(&self, email: &str) -> Result<()> {
        for slot in ["master-key", "tokens"] {
            match Self::entry(email, slot)?.delete_credential() {
                Ok(()) => {}
                Err(keyring::Error::NoEntry) => {}
                Err(e) => return Err(anyhow::anyhow!(e)),
            }
        }
        Ok(())
    }
}

// ---------- FileStore ----------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct FileStoreFile {
    entries: std::collections::BTreeMap<String, FileStoreEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileStoreEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    master_key_b64: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tokens: Option<AuthTokens>,
}

pub struct FileStore {
    path: PathBuf,
}

impl FileStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn default_path() -> Result<PathBuf> {
        let dir = dirs::config_dir().context("could not resolve user config dir")?;
        Ok(dir.join("confctl").join("vault-secrets.toml"))
    }

    fn load(&self) -> Result<FileStoreFile> {
        if !self.path.exists() {
            return Ok(FileStoreFile::default());
        }
        let body = std::fs::read_to_string(&self.path)
            .with_context(|| format!("reading {}", self.path.display()))?;
        if body.trim().is_empty() {
            return Ok(FileStoreFile::default());
        }
        toml::from_str(&body).with_context(|| format!("parsing {}", self.path.display()))
    }

    fn save(&self, file: &FileStoreFile) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let body = toml::to_string_pretty(file).context("serializing secret store")?;
        std::fs::write(&self.path, body)
            .with_context(|| format!("writing {}", self.path.display()))?;
        set_user_only_mode(&self.path)?;
        Ok(())
    }
}

impl SecretStore for FileStore {
    fn put_master_key(&self, email: &str, key: &[u8; KEY_LEN]) -> Result<()> {
        let mut file = self.load()?;
        let entry = file
            .entries
            .entry(email.to_string())
            .or_insert_with(|| FileStoreEntry {
                master_key_b64: None,
                tokens: None,
            });
        entry.master_key_b64 = Some(STANDARD.encode(key));
        self.save(&file)
    }

    fn get_master_key(&self, email: &str) -> Result<Option<[u8; KEY_LEN]>> {
        let file = self.load()?;
        let Some(entry) = file.entries.get(email) else {
            return Ok(None);
        };
        let Some(b64) = entry.master_key_b64.as_deref() else {
            return Ok(None);
        };
        let bytes = STANDARD
            .decode(b64)
            .context("decoding master-key from file store")?;
        if bytes.len() != KEY_LEN {
            anyhow::bail!("master-key wrong length in file store");
        }
        let mut out = [0u8; KEY_LEN];
        out.copy_from_slice(&bytes);
        Ok(Some(out))
    }

    fn put_tokens(&self, email: &str, tokens: &AuthTokens) -> Result<()> {
        let mut file = self.load()?;
        let entry = file
            .entries
            .entry(email.to_string())
            .or_insert_with(|| FileStoreEntry {
                master_key_b64: None,
                tokens: None,
            });
        entry.tokens = Some(tokens.clone());
        self.save(&file)
    }

    fn get_tokens(&self, email: &str) -> Result<Option<AuthTokens>> {
        let file = self.load()?;
        Ok(file.entries.get(email).and_then(|e| e.tokens.clone()))
    }

    fn clear(&self, email: &str) -> Result<()> {
        let mut file = self.load()?;
        file.entries.remove(email);
        self.save(&file)
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
    use chrono::TimeZone;

    fn tempfile(tag: &str) -> PathBuf {
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("confctl-vault-secrets-{tag}-{pid}-{nanos}.toml"))
    }

    fn sample_tokens() -> AuthTokens {
        AuthTokens {
            access: "access-jwt".into(),
            refresh: "refresh-opaque".into(),
            expires_at: Utc.with_ymd_and_hms(2026, 4, 23, 12, 0, 0).unwrap(),
        }
    }

    #[test]
    fn file_store_roundtrip() {
        let path = tempfile("roundtrip");
        let store = FileStore::new(path.clone());

        let key = [0x42u8; KEY_LEN];
        store.put_master_key("me@a.b", &key).unwrap();
        store.put_tokens("me@a.b", &sample_tokens()).unwrap();

        let got_key = store.get_master_key("me@a.b").unwrap().unwrap();
        assert_eq!(got_key, key);

        let got_tok = store.get_tokens("me@a.b").unwrap().unwrap();
        assert_eq!(got_tok.refresh, "refresh-opaque");

        // Second account is isolated
        assert!(store.get_master_key("other@a.b").unwrap().is_none());

        store.clear("me@a.b").unwrap();
        assert!(store.get_master_key("me@a.b").unwrap().is_none());
        assert!(store.get_tokens("me@a.b").unwrap().is_none());

        std::fs::remove_file(&path).ok();
    }

    #[cfg(unix)]
    #[test]
    fn file_store_is_0600() {
        use std::os::unix::fs::PermissionsExt;
        let path = tempfile("mode");
        let store = FileStore::new(path.clone());
        store.put_master_key("m@x", &[1u8; KEY_LEN]).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "expected 0600, got {mode:o}");

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn file_store_missing_returns_none() {
        let path = tempfile("missing");
        let store = FileStore::new(path);
        assert!(store.get_master_key("nobody@x").unwrap().is_none());
        assert!(store.get_tokens("nobody@x").unwrap().is_none());
    }
}
