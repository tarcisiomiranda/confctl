//! HashiCorp Vault KV v2 backend.
//!
//! Wire contract:
//!   - Base: `$VAULT_ADDR` (e.g. https://vault.example.com:8200). The REST
//!     prefix is `/v1/`, then the KV v2 mount (default `secret`).
//!   - Auth header: `X-Vault-Token: <token>`.
//!   - Enterprise namespace header: `X-Vault-Namespace: <ns>` (optional).
//!
//! UX parity with the other backends:
//!   - `push`  → `POST   /v1/<mount>/data/<name>`     creates a new version
//!   - `pull`  → `GET    /v1/<mount>/data/<name>`
//!   - `list`  → `LIST   /v1/<mount>/metadata/`       (HTTP verb `LIST`)
//!   - `rm`    → `DELETE /v1/<mount>/metadata/<name>` (wipes every version)
//!
//! Bytes travel as `{"data": {"content": base64, "filename": ..., "mime_type": ...}}`
//! — the KV store sees structured key/value pairs, which matches how Terraform
//! and Nomad consume Vault. The server-side encryption at rest is Vault's
//! own seal. We do NOT do client-side crypto here (see /plans/ decision).
//!
//! Auth discovery order:
//!   1. Cached token in the SecretStore (set by `vault login --backend hcp`).
//!   2. `hcp.token_file` from `vault.toml` (typically `/etc/confctl/hcp-token`).
//!   3. `VAULT_TOKEN` env var.
//!   4. `~/.vault-token` (the CLI's default).
//!
//! The first match wins; if none yields a token, calls fail with a clear hint.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::super::config::{BackendKind, HcpConfig, VaultConfig};
use super::super::storage::{default_store, SecretStore};
use super::{
    Backend, BackendStatus, Entry, LoginOpts, PullRequest, PulledSecret, PushRequest, SessionStatus,
};

const DEFAULT_MOUNT: &str = "secret";
const KEYRING_SLOT_TOKEN: &str = "token";

pub struct HcpBackend {
    cfg_path: PathBuf,
    cfg: VaultConfig,
    store: Box<dyn SecretStore>,
    agent: ureq::Agent,
    /// Cached after first resolve — never written back to disk.
    token: Option<String>,
}

impl HcpBackend {
    pub fn new(cfg_path: PathBuf, cfg: VaultConfig) -> Result<Self> {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(60))
            .user_agent(concat!("confctl/", env!("CARGO_PKG_VERSION")))
            .build();
        Ok(Self {
            cfg_path,
            cfg,
            store: default_store(),
            agent,
            token: None,
        })
    }

    fn hcp_cfg(&self) -> Result<&HcpConfig> {
        self.cfg.hcp.as_ref().ok_or_else(|| {
            anyhow!("hcp backend has no configuration. Run `confctl vault login --backend hcp --endpoint <VAULT_ADDR>`.")
        })
    }

    fn url(&self, path: &str) -> Result<String> {
        let addr = self.hcp_cfg()?.addr.trim_end_matches('/');
        Ok(format!("{addr}/v1{path}"))
    }

    /// Resolve the token to use for this invocation.
    fn resolve_token(&mut self) -> Result<String> {
        if let Some(t) = &self.token {
            return Ok(t.clone());
        }
        let account = self.token_account()?;
        if let Some(t) = self.store.get_tokens(&account)? {
            self.token = Some(t.access.clone());
            return Ok(t.access);
        }
        if let Some(path) = self.cfg.hcp.as_ref().and_then(|c| c.token_file.clone()) {
            let token = std::fs::read_to_string(&path)
                .with_context(|| format!("reading hcp.token_file at {}", path.display()))?;
            let token = token.trim().to_string();
            if !token.is_empty() {
                self.token = Some(token.clone());
                return Ok(token);
            }
        }
        if let Ok(t) = std::env::var("VAULT_TOKEN") {
            if !t.is_empty() {
                self.token = Some(t.clone());
                return Ok(t);
            }
        }
        if let Some(home) = dirs::home_dir() {
            let path = home.join(".vault-token");
            if path.exists() {
                let token = std::fs::read_to_string(&path)
                    .with_context(|| format!("reading {}", path.display()))?;
                let token = token.trim().to_string();
                if !token.is_empty() {
                    self.token = Some(token.clone());
                    return Ok(token);
                }
            }
        }
        bail!(
            "no Vault token available. Run `confctl vault login --backend hcp`, set VAULT_TOKEN, or point `hcp.token_file` at a token file."
        )
    }

    /// SecretStore account identifier for the cached token.
    fn token_account(&self) -> Result<String> {
        Ok(format!("hcp:{}", self.hcp_cfg()?.addr))
    }

    fn mount(&self) -> Result<&str> {
        Ok(self
            .cfg
            .hcp
            .as_ref()
            .map(|c| c.mount.as_str())
            .unwrap_or(DEFAULT_MOUNT))
    }

    fn apply_auth(&mut self, mut req: ureq::Request) -> Result<ureq::Request> {
        let token = self.resolve_token()?;
        req = req.set("X-Vault-Token", &token);
        if let Some(ns) = self.cfg.hcp.as_ref().and_then(|c| c.namespace.as_deref()) {
            req = req.set("X-Vault-Namespace", ns);
        }
        Ok(req)
    }

    /// Validate a token via `/v1/auth/token/lookup-self`. Returns the
    /// display_name from the token for the status line.
    fn lookup_self(&mut self, token: &str) -> Result<LookupSelf> {
        let url = self.url("/auth/token/lookup-self")?;
        let mut req = self.agent.get(&url).set("X-Vault-Token", token);
        if let Some(ns) = self.cfg.hcp.as_ref().and_then(|c| c.namespace.as_deref()) {
            req = req.set("X-Vault-Namespace", ns);
        }
        let resp = req.call().map_err(map_hcp_err)?;
        let body: LookupSelfEnvelope = resp
            .into_json()
            .context("parsing /auth/token/lookup-self response")?;
        Ok(body.data)
    }
}

// ---------- Wire types ----------

#[derive(Debug, Deserialize)]
struct LookupSelfEnvelope {
    data: LookupSelf,
}

#[derive(Debug, Clone, Deserialize)]
struct LookupSelf {
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    ttl: Option<i64>,
    #[serde(default)]
    expire_time: Option<String>,
    #[serde(default)]
    renewable: Option<bool>,
}

#[derive(Debug, Serialize)]
struct KvData<'a> {
    content: &'a str,
    filename: &'a str,
    mime_type: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    labels: Option<&'a [String]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    notes: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct KvWriteBody<'a> {
    data: KvData<'a>,
}

#[derive(Debug, Deserialize)]
struct KvReadEnvelope {
    data: KvReadData,
}

#[derive(Debug, Deserialize)]
struct KvReadData {
    data: KvStoredPayload,
    metadata: KvReadMetadata,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct KvReadMetadata {
    created_time: Option<String>,
    custom_metadata: Option<serde_json::Value>,
    deletion_time: Option<String>,
    destroyed: Option<bool>,
    version: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct KvStoredPayload {
    content: String,
    #[serde(default)]
    filename: Option<String>,
    #[serde(default)]
    mime_type: Option<String>,
    #[serde(default)]
    labels: Option<Vec<String>>,
    #[serde(default)]
    notes: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ListEnvelope {
    data: ListData,
}

#[derive(Debug, Deserialize)]
struct ListData {
    keys: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct MetadataEnvelope {
    data: MetadataData,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct MetadataData {
    created_time: Option<String>,
    updated_time: Option<String>,
    current_version: Option<u64>,
}

// ---------- Backend impl ----------

impl Backend for HcpBackend {
    fn kind(&self) -> &'static str {
        "hcp"
    }

    fn login(&mut self, opts: LoginOpts) -> Result<()> {
        let addr = match opts.endpoint {
            Some(u) => u,
            None => std::env::var("VAULT_ADDR")
                .ok()
                .filter(|s| !s.is_empty())
                .map(Ok)
                .unwrap_or_else(|| prompt_line("Vault addr (VAULT_ADDR): "))?,
        };
        let addr = addr.trim_end_matches('/').to_string();

        let mount = opts.identity.unwrap_or_else(|| DEFAULT_MOUNT.to_string());

        let token_env = std::env::var("VAULT_TOKEN").ok().filter(|s| !s.is_empty());
        let token = match token_env {
            Some(t) => t,
            None => rpassword::prompt_password("Vault token: ").context("reading token")?,
        };
        if token.is_empty() {
            bail!("empty Vault token");
        }

        // Stash config before validating so the URL helpers work.
        self.cfg.hcp = Some(HcpConfig {
            addr: addr.clone(),
            mount: mount.clone(),
            namespace: self.cfg.hcp.as_ref().and_then(|c| c.namespace.clone()),
            token_file: self.cfg.hcp.as_ref().and_then(|c| c.token_file.clone()),
        });
        self.cfg.backend = Some(BackendKind::Hcp);

        let info = self
            .lookup_self(&token)
            .context("validating token against /auth/token/lookup-self")?;

        self.cfg
            .save_to(&self.cfg_path)
            .with_context(|| format!("writing vault config to {}", self.cfg_path.display()))?;

        let account = self.token_account()?;
        let expires_at = parse_hcp_expire(&info);
        let tokens = super::super::storage::AuthTokens {
            access: token.clone(),
            refresh: String::new(),
            expires_at,
        };
        self.store
            .put_tokens(&account, &tokens)
            .context("storing Vault token in keyring")?;

        self.token = Some(token);

        println!(
            "{} logged in against {} (mount={}, display_name={})",
            colored::Colorize::green(colored::Colorize::bold("✓")),
            colored::Colorize::bold(addr.as_str()),
            mount,
            info.display_name.as_deref().unwrap_or("-")
        );
        Ok(())
    }

    fn logout(&mut self) -> Result<()> {
        // Best-effort: revoke on server if we can.
        if let Ok(token) = self.resolve_token() {
            let url = self.url("/auth/token/revoke-self")?;
            let mut req = self.agent.post(&url).set("X-Vault-Token", &token);
            if let Some(ns) = self.cfg.hcp.as_ref().and_then(|c| c.namespace.as_deref()) {
                req = req.set("X-Vault-Namespace", ns);
            }
            if let Err(err) = req.call().map_err(map_hcp_err) {
                eprintln!(
                    "warning: server-side token revocation failed ({err}); clearing local session anyway."
                );
            }
        }
        if let Ok(account) = self.token_account() {
            self.store
                .clear(&account)
                .context("clearing Vault token from keyring")?;
        }
        self.token = None;
        Ok(())
    }

    fn status(&self) -> Result<BackendStatus> {
        let Some(hcp) = &self.cfg.hcp else {
            return Ok(BackendStatus {
                kind: "hcp",
                endpoint: String::new(),
                identity: None,
                session: SessionStatus::Missing,
                master_key_cached: None,
            });
        };
        let account = format!("hcp:{}", hcp.addr);
        let tokens = self.store.get_tokens(&account)?;
        let session = match tokens {
            Some(t) if t.expires_at > Utc::now() => SessionStatus::Valid {
                expires_at: Some(t.expires_at),
            },
            Some(_) => SessionStatus::Expired,
            None => {
                // No cached token — might still work via token_file, env var,
                // or ~/.vault-token.
                let token_file_usable =
                    hcp.token_file.as_ref().map(|p| p.exists()).unwrap_or(false);
                let env_set = std::env::var("VAULT_TOKEN")
                    .ok()
                    .is_some_and(|v| !v.is_empty());
                let home_file = dirs::home_dir()
                    .map(|h| h.join(".vault-token").exists())
                    .unwrap_or(false);
                if token_file_usable || env_set || home_file {
                    SessionStatus::Ambient
                } else {
                    SessionStatus::Missing
                }
            }
        };
        Ok(BackendStatus {
            kind: "hcp",
            endpoint: format!("{} (mount={})", hcp.addr, hcp.mount),
            identity: hcp.namespace.clone(),
            session,
            master_key_cached: None,
        })
    }

    fn list(&mut self) -> Result<Vec<Entry>> {
        let mount = self.mount()?.to_string();
        let url = self.url(&format!("/{mount}/metadata/"))?;
        let req = self.apply_auth(self.agent.request("LIST", &url))?;
        let resp = match req.call() {
            Ok(r) => r,
            // Vault returns 404 when the metadata path has never been written to.
            Err(ureq::Error::Status(404, _)) => return Ok(Vec::new()),
            Err(e) => return Err(map_hcp_err(e)),
        };
        let body: ListEnvelope = resp.into_json().context("parsing LIST metadata response")?;

        let mut out = Vec::with_capacity(body.data.keys.len());
        for key in body.data.keys {
            let meta = self.fetch_metadata(&mount, &key).ok();
            let updated_at = meta
                .as_ref()
                .and_then(|m| m.updated_time.as_deref())
                .or_else(|| meta.as_ref().and_then(|m| m.created_time.as_deref()))
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|d| d.with_timezone(&Utc));
            out.push(Entry {
                id: meta
                    .and_then(|m| m.current_version)
                    .map(|v| format!("v{v}"))
                    .unwrap_or_default(),
                name: key,
                size: None,
                updated_at,
                labels: None,
                filename: None,
            });
        }
        Ok(out)
    }

    fn push(&mut self, req: PushRequest<'_>) -> Result<Entry> {
        let mount = self.mount()?.to_string();
        let name = &req.name;

        // Respect --overwrite by checking existence first. (Vault KV v2 will
        // silently create a new version on a bare write — fine for overwrite,
        // wrong for "reject duplicate".)
        if !req.overwrite {
            let probe_url = self.url(&format!("/{mount}/metadata/{name}"))?;
            let probe = self.apply_auth(self.agent.get(&probe_url))?;
            match probe.call() {
                Ok(_) => {
                    bail!("a secret named {name:?} already exists in `{mount}`; pass --overwrite to replace it")
                }
                Err(ureq::Error::Status(404, _)) => {}
                Err(e) => return Err(map_hcp_err(e)),
            }
        }

        let content_b64 = STANDARD.encode(req.bytes);
        let filename = req.filename.as_deref().unwrap_or(name);
        let mime = req.mime.as_deref().unwrap_or("application/octet-stream");
        let labels = if req.labels.is_empty() {
            None
        } else {
            Some(req.labels.as_slice())
        };
        let body = KvWriteBody {
            data: KvData {
                content: &content_b64,
                filename,
                mime_type: mime,
                labels,
                notes: None,
            },
        };

        let url = self.url(&format!("/{mount}/data/{name}"))?;
        let resp = self
            .apply_auth(self.agent.post(&url))?
            .send_json(&body)
            .map_err(map_hcp_err)?;

        // Response carries the new version number under data.version.
        let envelope: serde_json::Value =
            resp.into_json().context("parsing POST /data response")?;
        let version = envelope
            .get("data")
            .and_then(|d| d.get("version"))
            .and_then(|v| v.as_u64())
            .map(|v| format!("v{v}"))
            .unwrap_or_default();

        Ok(Entry {
            id: version,
            name: name.clone(),
            size: Some(req.bytes.len() as u64),
            updated_at: Some(Utc::now()),
            labels: None,
            filename: Some(filename.to_string()),
        })
    }

    fn pull(&mut self, req: PullRequest<'_>) -> Result<PulledSecret> {
        let name = req
            .name
            .or(req.id)
            .ok_or_else(|| anyhow!("provide a display name or --id"))?;
        let mount = self.mount()?.to_string();
        let url = self.url(&format!("/{mount}/data/{name}"))?;
        let resp = self
            .apply_auth(self.agent.get(&url))?
            .call()
            .map_err(map_hcp_err)?;
        let env: KvReadEnvelope = resp.into_json().context("parsing GET /data response")?;
        let payload = env.data.data;
        let bytes = STANDARD
            .decode(&payload.content)
            .context("decoding base64 content from Vault")?;
        Ok(PulledSecret {
            bytes,
            filename: payload.filename.or_else(|| Some(name.to_string())),
        })
    }

    fn rm(&mut self, name: &str) -> Result<()> {
        let mount = self.mount()?.to_string();
        let url = self.url(&format!("/{mount}/metadata/{name}"))?;
        self.apply_auth(self.agent.delete(&url))?
            .call()
            .map_err(map_hcp_err)?;
        Ok(())
    }
}

impl HcpBackend {
    fn fetch_metadata(&mut self, mount: &str, name: &str) -> Result<MetadataData> {
        let url = self.url(&format!("/{mount}/metadata/{name}"))?;
        let resp = self
            .apply_auth(self.agent.get(&url))?
            .call()
            .map_err(map_hcp_err)?;
        let env: MetadataEnvelope = resp.into_json().context("parsing metadata response")?;
        Ok(env.data)
    }
}

// ---------- Error mapping ----------

fn map_hcp_err(err: ureq::Error) -> anyhow::Error {
    match err {
        ureq::Error::Status(status, resp) => {
            let body = resp
                .into_string()
                .unwrap_or_else(|_| format!("HTTP {status}"));
            // Vault returns JSON like {"errors": ["..."]} on error.
            let pretty = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| {
                    v.get("errors").and_then(|e| e.as_array()).map(|arr| {
                        arr.iter()
                            .filter_map(|x| x.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                })
                .filter(|s| !s.is_empty())
                .unwrap_or(body);
            anyhow!("Vault returned {status}: {pretty}")
        }
        ureq::Error::Transport(t) => anyhow!("transport error against Vault: {t}"),
    }
}

fn parse_hcp_expire(info: &LookupSelf) -> DateTime<Utc> {
    // Prefer explicit expire_time (ISO 8601). Fall back to `ttl` seconds
    // from now. Root tokens have neither — use a far-future sentinel.
    if let Some(et) = &info.expire_time {
        if let Ok(parsed) = DateTime::parse_from_rfc3339(et) {
            return parsed.with_timezone(&Utc);
        }
    }
    if let Some(ttl) = info.ttl {
        if ttl > 0 {
            return Utc::now() + chrono::Duration::seconds(ttl);
        }
    }
    // Root / never-expires.
    Utc::now() + chrono::Duration::days(3650)
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

#[allow(dead_code)]
fn config_path_placeholder() -> &'static Path {
    Path::new("/unused")
}
