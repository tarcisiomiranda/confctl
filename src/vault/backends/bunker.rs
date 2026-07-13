//! Bunker Vault backend — zero-knowledge client-side encryption.
//!
//! This is the original implementation, moved here behind the `Backend`
//! trait. The CLI in `vault::cli` no longer speaks bunker directly — it
//! constructs this type via `vault::backends::open` and calls the trait.

use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};

use super::super::client::{is_unauthorized, HttpClient, SecretResponseDto};
use super::super::config::{BackendKind, BunkerConfig, VaultConfig};
use super::super::crypto::{self, auth_hash, derive_master_key, SecretEnvelope};
use super::super::storage::{default_store, AuthTokens, SecretStore};
use super::{
    Backend, BackendStatus, Entry, LoginOpts, PullRequest, PulledSecret, PushRequest, SessionStatus,
};

const KIND_FILE: &str = "file";

// ---------- Plaintext payload (matches vault-types::Secret::File) ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
enum SecretPayload {
    File(FilePayload),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FilePayload {
    filename: String,
    mime_type: String,
    size: u64,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    labels: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    notes: Option<String>,
}

impl FilePayload {
    fn identifier(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.filename)
    }
}

// ---------- Backend ----------

pub struct BunkerBackend {
    cfg_path: PathBuf,
    cfg: VaultConfig,
    store: Box<dyn SecretStore>,
    /// Lazily filled on the first call that needs a session.
    session: Option<Session>,
}

struct Session {
    client: HttpClient,
    tokens: AuthTokens,
    master_key: [u8; crypto::KEY_LEN],
}

impl BunkerBackend {
    pub fn new(cfg_path: PathBuf, cfg: VaultConfig) -> Result<Self> {
        Ok(Self {
            cfg_path,
            cfg,
            store: default_store(),
            session: None,
        })
    }

    fn bunker_cfg(&self) -> Result<&BunkerConfig> {
        self.cfg.bunker.as_ref().ok_or_else(|| {
            anyhow!(
                "bunker backend has no configuration. Run `confctl vault login --backend bunker`."
            )
        })
    }

    fn require_session(&mut self) -> Result<&mut Session> {
        if self.session.is_none() {
            let bunker = self.bunker_cfg()?.clone();
            let tokens = self
                .store
                .get_tokens(&bunker.email)
                .context("loading bunker tokens")?
                .ok_or_else(|| {
                    anyhow!("no cached tokens; run `confctl vault login --backend bunker`")
                })?;
            let master_key = self
                .store
                .get_master_key(&bunker.email)
                .context("loading bunker master-key")?
                .ok_or_else(|| {
                    anyhow!("no cached master-key; run `confctl vault login --backend bunker`")
                })?;
            let client = HttpClient::new(&bunker.url);
            self.session = Some(Session {
                client,
                tokens,
                master_key,
            });
        }
        Ok(self.session.as_mut().unwrap())
    }

    /// Run a remote op with automatic refresh on 401. The new refresh token
    /// is persisted to the SecretStore *before* the retry, so a crash between
    /// calls can't lock the account out.
    fn call<T>(&mut self, op: impl Fn(&HttpClient, &str) -> Result<T>) -> Result<T> {
        let session = self.require_session()?;
        match op(&session.client, &session.tokens.access) {
            Ok(v) => Ok(v),
            Err(e) if is_unauthorized(&e) => {
                let new_tokens = session
                    .client
                    .refresh(&session.tokens.refresh)
                    .context("refreshing bunker session (old refresh token may be expired)")?;
                let email = self.bunker_cfg()?.email.clone();
                self.store
                    .put_tokens(&email, &new_tokens)
                    .context("persisting refreshed tokens")?;
                let session = self.session.as_mut().expect("session present");
                session.tokens = new_tokens;
                op(&session.client, &session.tokens.access)
            }
            Err(e) => Err(e),
        }
    }

    fn list_files(&mut self) -> Result<Vec<(SecretResponseDto, FilePayload)>> {
        let items = self.call(|c, access| c.list_secrets(access))?;
        let master_key = self
            .session
            .as_ref()
            .expect("session initialized by list_secrets")
            .master_key;
        let mut out = Vec::new();
        for item in items {
            if item.kind != KIND_FILE {
                continue;
            }
            let (ct, nonce) = item.decode()?;
            let Ok(plain) = crypto::decrypt(&master_key, &ct, &nonce) else {
                eprintln!(
                    "warning: failed to decrypt bunker secret {} (skipping)",
                    item.id
                );
                continue;
            };
            let payload: SecretPayload = match serde_json::from_slice(&plain) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!(
                        "warning: bunker secret {} has unexpected plaintext ({e}); skipping",
                        item.id
                    );
                    continue;
                }
            };
            let SecretPayload::File(fp) = payload;
            out.push((item, fp));
        }
        Ok(out)
    }
}

impl Backend for BunkerBackend {
    fn kind(&self) -> &'static str {
        "bunker"
    }

    fn login(&mut self, opts: LoginOpts) -> Result<()> {
        let url = match opts.endpoint {
            Some(u) => u,
            None => prompt_line("Vault URL: ")?,
        };
        let email = match opts.identity {
            Some(e) => e,
            None => prompt_line("Email: ")?,
        };
        let password = match std::env::var("CONFCTL_VAULT_PASSWORD") {
            Ok(p) if !p.is_empty() => p,
            _ => rpassword::prompt_password("Password: ").context("reading password")?,
        };

        let client = HttpClient::new(&url);
        let salt = client
            .get_salt(&email)
            .context("requesting kdf salt from vault")?;

        let master_key = derive_master_key(password.as_bytes(), &salt)
            .context("deriving master key with Argon2id")?;
        let hash = auth_hash(&master_key, &salt).context("computing auth hash")?;

        let tokens = client
            .login(&email, &hash)
            .context("authenticating against vault")?;

        // Persist config: select bunker as the active backend and store its
        // per-backend section.
        self.cfg.backend = Some(BackendKind::Bunker);
        self.cfg.bunker = Some(BunkerConfig {
            url: url.clone(),
            email: email.clone(),
            kdf_salt_b64: STANDARD.encode(&salt),
        });
        self.cfg
            .save_to(&self.cfg_path)
            .with_context(|| format!("writing vault config to {}", self.cfg_path.display()))?;

        self.store
            .put_master_key(&email, &master_key)
            .context("storing master-key")?;
        self.store
            .put_tokens(&email, &tokens)
            .context("storing tokens")?;

        // Prime the in-memory session so a subsequent list/push call in the
        // same process reuses the just-derived credentials.
        self.session = Some(Session {
            client,
            tokens: tokens.clone(),
            master_key: *master_key,
        });

        println!(
            "{} logged in as {} against {} (session valid until {})",
            colored::Colorize::green(colored::Colorize::bold("✓")),
            colored::Colorize::bold(email.as_str()),
            colored::Colorize::bold(url.as_str()),
            tokens.expires_at.to_rfc3339()
        );
        Ok(())
    }

    fn logout(&mut self) -> Result<()> {
        let Some(bunker) = self.cfg.bunker.clone() else {
            println!("(no bunker session to log out from)");
            return Ok(());
        };

        if let Some(tokens) = self.store.get_tokens(&bunker.email)? {
            let client = HttpClient::new(&bunker.url);
            if let Err(err) = client.logout(&tokens.refresh, &tokens.access) {
                eprintln!(
                    "warning: server-side logout failed ({err}); clearing local session anyway."
                );
            }
        }
        self.store
            .clear(&bunker.email)
            .context("clearing local secret store")?;
        self.session = None;
        Ok(())
    }

    fn status(&self) -> Result<BackendStatus> {
        let Some(bunker) = &self.cfg.bunker else {
            return Ok(BackendStatus {
                kind: "bunker",
                endpoint: String::new(),
                identity: None,
                session: SessionStatus::Missing,
                master_key_cached: Some(false),
            });
        };
        let tokens = self.store.get_tokens(&bunker.email)?;
        let master_key_cached = self.store.get_master_key(&bunker.email)?.is_some();

        let session = match tokens {
            Some(t) if t.expires_at > chrono::Utc::now() => SessionStatus::Valid {
                expires_at: Some(t.expires_at),
            },
            Some(_) => SessionStatus::Expired,
            None => SessionStatus::Missing,
        };

        Ok(BackendStatus {
            kind: "bunker",
            endpoint: bunker.url.clone(),
            identity: Some(bunker.email.clone()),
            session,
            master_key_cached: Some(master_key_cached),
        })
    }

    fn list(&mut self) -> Result<Vec<Entry>> {
        let items = self.list_files()?;
        Ok(items
            .into_iter()
            .map(|(dto, fp)| Entry {
                id: dto.id,
                name: fp.identifier().to_string(),
                size: Some(fp.size),
                updated_at: chrono::DateTime::parse_from_rfc3339(&dto.updated_at)
                    .ok()
                    .map(|d| d.with_timezone(&chrono::Utc)),
                labels: fp.labels,
                filename: Some(fp.filename),
            })
            .collect())
    }

    fn push(&mut self, req: PushRequest<'_>) -> Result<Entry> {
        let display_name = req.name;
        let filename = req.filename.clone().unwrap_or_else(|| display_name.clone());

        let existing = self
            .list_files()?
            .into_iter()
            .find(|(_, fp)| fp.identifier() == display_name);

        if existing.is_some() && !req.overwrite {
            bail!("a secret named {display_name:?} already exists; pass --overwrite to replace it");
        }

        let payload = FilePayload {
            filename,
            mime_type: req
                .mime
                .unwrap_or_else(|| "application/octet-stream".into()),
            size: req.bytes.len() as u64,
            content: STANDARD.encode(req.bytes),
            display_name: Some(display_name.clone()),
            labels: if req.labels.is_empty() {
                None
            } else {
                Some(req.labels)
            },
            notes: None,
        };

        let plain =
            serde_json::to_vec(&SecretPayload::File(payload)).context("serializing payload")?;
        let session = self.require_session()?;
        let env: SecretEnvelope =
            crypto::encrypt(&session.master_key, &plain).context("encrypting payload")?;

        let dto = if let Some((dto, _)) = existing {
            let id = dto.id.clone();
            self.call(|c, access| c.update_secret(access, &id, KIND_FILE, &env))?
        } else {
            self.call(|c, access| c.create_secret(access, KIND_FILE, &env))?
        };

        Ok(Entry {
            id: dto.id,
            name: display_name,
            size: Some(req.bytes.len() as u64),
            updated_at: None,
            labels: None,
            filename: None,
        })
    }

    fn pull(&mut self, req: PullRequest<'_>) -> Result<PulledSecret> {
        if req.name.is_none() && req.id.is_none() {
            bail!("provide a display name or --id");
        }

        let items = self.list_files()?;
        let hit = if let Some(id) = req.id {
            items.into_iter().find(|(dto, _)| dto.id == id)
        } else if let Some(name) = req.name {
            let matches: Vec<_> = items
                .into_iter()
                .filter(|(_, fp)| fp.identifier() == name)
                .collect();
            match matches.len() {
                0 => None,
                1 => Some(matches.into_iter().next().unwrap()),
                n => bail!("{n} secrets match name {name:?}; disambiguate with --id"),
            }
        } else {
            None
        };

        let (_, payload) = hit.ok_or_else(|| anyhow!("no matching secret"))?;
        let bytes = STANDARD
            .decode(&payload.content)
            .context("decoding payload content")?;
        Ok(PulledSecret {
            bytes,
            filename: Some(payload.identifier().to_string()),
        })
    }

    fn rm(&mut self, name: &str) -> Result<()> {
        let items = self.list_files()?;
        let matches: Vec<_> = items
            .into_iter()
            .filter(|(_, fp)| fp.identifier() == name)
            .collect();

        let dto = match matches.len() {
            0 => bail!("no secret named {name:?}"),
            1 => matches.into_iter().next().unwrap().0,
            n => bail!("{n} secrets match name {name:?}; aborting"),
        };

        let id = dto.id.clone();
        self.call(|c, access| c.delete_secret(access, &id))?;
        Ok(())
    }
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
