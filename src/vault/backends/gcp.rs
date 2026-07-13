//! Google Secret Manager backend.
//!
//! Wire contract (REST v1):
//!   - Base: `https://secretmanager.googleapis.com/v1`
//!   - `push` → `POST /projects/{p}/secrets?secretId={name}` (create, 409 on
//!     duplicate) then `POST /projects/{p}/secrets/{name}:addVersion`
//!   - `pull` → `GET  /projects/{p}/secrets/{name}/versions/latest:access`
//!   - `list` → `GET  /projects/{p}/secrets` (paginated)
//!   - `rm`   → `DELETE /projects/{p}/secrets/{name}`
//!
//! Filename / mime / labels ride along as secret **annotations** so `pull`
//! can restore the original filename. Secret Manager encrypts at rest with
//! Google-managed (or CMEK) keys — no client-side crypto here, matching the
//! hcp backend decision.
//!
//! Auth is ambient ADC — no token is ever cached by confctl. Discovery order:
//!   1. `CLOUDSDK_AUTH_ACCESS_TOKEN` env var (gcloud's own override).
//!   2. Credentials JSON (authorized_user): `gcp.credentials_file` from
//!      vault.toml → `GOOGLE_APPLICATION_CREDENTIALS` → the gcloud ADC file
//!      at ~/.config/gcloud/application_default_credentials.json. The
//!      refresh_token is exchanged at https://oauth2.googleapis.com/token.
//!   3. `gcloud auth application-default print-access-token`, then
//!      `gcloud auth print-access-token` (covers service accounts too).
//!   4. GCE metadata server (inside GCP compute).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;

use super::super::config::{BackendKind, GcpConfig, VaultConfig};
use super::{
    Backend, BackendStatus, Entry, LoginOpts, PullRequest, PulledSecret, PushRequest, SessionStatus,
};

const API_BASE: &str = "https://secretmanager.googleapis.com/v1";
const OAUTH_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const METADATA_TOKEN_URL: &str =
    "http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token";

pub struct GcpBackend {
    cfg_path: PathBuf,
    cfg: VaultConfig,
    agent: ureq::Agent,
    /// Cached for this invocation only — never persisted.
    token: Option<String>,
}

impl GcpBackend {
    pub fn new(cfg_path: PathBuf, cfg: VaultConfig) -> Result<Self> {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(60))
            .user_agent(concat!("confctl/", env!("CARGO_PKG_VERSION")))
            .build();
        Ok(Self {
            cfg_path,
            cfg,
            agent,
            token: None,
        })
    }

    fn gcp_cfg(&self) -> Result<&GcpConfig> {
        self.cfg.gcp.as_ref().ok_or_else(|| {
            anyhow!("gcp backend has no configuration. Run `confctl vault login --backend gcp --endpoint <PROJECT_ID>`.")
        })
    }

    fn project(&self) -> Result<&str> {
        Ok(self.gcp_cfg()?.project.as_str())
    }

    fn url(&self, path: &str) -> Result<String> {
        Ok(format!("{API_BASE}/projects/{}{path}", self.project()?))
    }

    fn resolve_token(&mut self) -> Result<String> {
        if let Some(t) = &self.token {
            return Ok(t.clone());
        }
        let token = self.discover_token()?;
        self.token = Some(token.clone());
        Ok(token)
    }

    fn discover_token(&self) -> Result<String> {
        if let Ok(t) = std::env::var("CLOUDSDK_AUTH_ACCESS_TOKEN") {
            if !t.is_empty() {
                return Ok(t);
            }
        }
        if let Some(path) = self.adc_credentials_path() {
            match self.token_from_adc_file(&path) {
                Ok(t) => return Ok(t),
                Err(err) => eprintln!(
                    "warning: ADC credentials at {} unusable ({err}); trying gcloud CLI.",
                    path.display()
                ),
            }
        }
        let gcloud_arg_sets: [&[&str]; 2] = [
            &["auth", "application-default", "print-access-token"],
            &["auth", "print-access-token"],
        ];
        for args in gcloud_arg_sets {
            if let Some(t) = gcloud_token(args) {
                return Ok(t);
            }
        }
        if let Ok(t) = self.token_from_metadata_server() {
            return Ok(t);
        }
        bail!(
            "no Google credentials found. Run `gcloud auth application-default login`, \
             set GOOGLE_APPLICATION_CREDENTIALS, or run inside GCP."
        )
    }

    fn adc_credentials_path(&self) -> Option<PathBuf> {
        if let Some(p) = self.cfg.gcp.as_ref().and_then(|c| c.credentials_file.clone()) {
            return Some(p);
        }
        if let Ok(p) = std::env::var("GOOGLE_APPLICATION_CREDENTIALS") {
            if !p.is_empty() {
                return Some(PathBuf::from(p));
            }
        }
        let default = dirs::config_dir()?
            .join("gcloud")
            .join("application_default_credentials.json");
        default.exists().then_some(default)
    }

    /// Exchange an `authorized_user` ADC refresh_token for an access token.
    /// Service-account JSONs need RS256 signing — those are served by the
    /// gcloud-CLI fallback instead.
    fn token_from_adc_file(&self, path: &Path) -> Result<String> {
        let body = std::fs::read_to_string(path)
            .with_context(|| format!("reading credentials at {}", path.display()))?;
        let creds: AdcCredentials = serde_json::from_str(&body)
            .with_context(|| format!("parsing credentials at {}", path.display()))?;
        if creds.credential_type.as_deref() != Some("authorized_user") {
            bail!(
                "credentials type {:?} is not `authorized_user`",
                creds.credential_type.as_deref().unwrap_or("unknown")
            );
        }
        let resp = self
            .agent
            .post(OAUTH_TOKEN_URL)
            .send_json(json!({
                "client_id": creds.client_id,
                "client_secret": creds.client_secret,
                "refresh_token": creds.refresh_token,
                "grant_type": "refresh_token",
            }))
            .map_err(map_gcp_err)?;
        let token: OauthTokenResponse = resp.into_json().context("parsing oauth token response")?;
        Ok(token.access_token)
    }

    fn token_from_metadata_server(&self) -> Result<String> {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(std::time::Duration::from_millis(500))
            .timeout(std::time::Duration::from_secs(3))
            .build();
        let resp = agent
            .get(METADATA_TOKEN_URL)
            .set("Metadata-Flavor", "Google")
            .call()
            .map_err(map_gcp_err)?;
        let token: OauthTokenResponse = resp
            .into_json()
            .context("parsing metadata server token response")?;
        Ok(token.access_token)
    }

    fn authed(&mut self, req: ureq::Request) -> Result<ureq::Request> {
        let token = self.resolve_token()?;
        Ok(req.set("Authorization", &format!("Bearer {token}")))
    }

    fn fetch_secret_meta(&mut self, name: &str) -> Result<GcpSecret> {
        let url = self.url(&format!("/secrets/{name}"))?;
        let resp = self
            .authed(self.agent.get(&url))?
            .call()
            .map_err(map_gcp_err)?;
        resp.into_json().context("parsing GET secret response")
    }
}

/// Secret Manager secret IDs must match `[A-Za-z0-9_-]{1,255}`. Anything
/// else (dots in `.env`, slashes, spaces) is folded to `-`.
pub fn sanitize_secret_id(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut last_dash = false;
    for c in raw.chars() {
        let mapped = if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            last_dash = c == '-';
            c
        } else if last_dash {
            continue;
        } else {
            last_dash = true;
            '-'
        };
        out.push(mapped);
    }
    let trimmed = out.trim_matches('-').to_string();
    let mut id = if trimmed.is_empty() {
        "secret".to_string()
    } else {
        trimmed
    };
    id.truncate(255);
    id
}

fn gcloud_token(args: &[&str]) -> Option<String> {
    let output = Command::new("gcloud").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let token = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (!token.is_empty()).then_some(token)
}

// ---------- Wire types ----------

#[derive(Debug, Deserialize)]
struct AdcCredentials {
    #[serde(rename = "type")]
    credential_type: Option<String>,
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    client_secret: String,
    #[serde(default)]
    refresh_token: String,
}

#[derive(Debug, Deserialize)]
struct OauthTokenResponse {
    access_token: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct GcpSecret {
    /// Full resource name: projects/{p}/secrets/{id}.
    name: String,
    #[serde(rename = "createTime")]
    create_time: Option<String>,
    annotations: Option<std::collections::BTreeMap<String, String>>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct ListSecretsResponse {
    secrets: Vec<GcpSecret>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AccessVersionResponse {
    payload: AccessPayload,
}

#[derive(Debug, Deserialize)]
struct AccessPayload {
    data: String,
}

fn short_id(resource_name: &str) -> String {
    resource_name
        .rsplit('/')
        .next()
        .unwrap_or(resource_name)
        .to_string()
}

// ---------- Backend impl ----------

impl Backend for GcpBackend {
    fn kind(&self) -> &'static str {
        "gcp"
    }

    fn login(&mut self, opts: LoginOpts) -> Result<()> {
        let project = match opts.endpoint.or(opts.identity) {
            Some(p) => p,
            None => std::env::var("GOOGLE_CLOUD_PROJECT")
                .or_else(|_| std::env::var("CLOUDSDK_CORE_PROJECT"))
                .ok()
                .filter(|s| !s.is_empty())
                .or_else(|| gcloud_token(&["config", "get-value", "project"]))
                .filter(|s| !s.is_empty() && s != "(unset)")
                .ok_or_else(|| {
                    anyhow!("no project given. Pass --endpoint <PROJECT_ID> or set GOOGLE_CLOUD_PROJECT.")
                })?,
        };

        self.cfg.gcp = Some(GcpConfig {
            project: project.clone(),
            credentials_file: self.cfg.gcp.as_ref().and_then(|c| c.credentials_file.clone()),
        });
        self.cfg.backend = Some(BackendKind::Gcp);

        // Validate ADC + project before persisting: one cheap list call.
        let url = self.url("/secrets?pageSize=1")?;
        self.authed(self.agent.get(&url))?
            .call()
            .map_err(map_gcp_err)
            .with_context(|| format!("validating Secret Manager access on project {project}"))?;

        self.cfg
            .save_to(&self.cfg_path)
            .with_context(|| format!("writing vault config to {}", self.cfg_path.display()))?;

        println!(
            "{} using project {} with ambient Google credentials (ADC)",
            colored::Colorize::green(colored::Colorize::bold("✓")),
            colored::Colorize::bold(project.as_str()),
        );
        Ok(())
    }

    fn logout(&mut self) -> Result<()> {
        // Nothing cached by confctl — credentials belong to gcloud/ADC.
        self.token = None;
        println!("note: credentials are managed by gcloud; run `gcloud auth application-default revoke` to revoke them.");
        Ok(())
    }

    fn status(&self) -> Result<BackendStatus> {
        let Some(gcp) = &self.cfg.gcp else {
            return Ok(BackendStatus {
                kind: "gcp",
                endpoint: String::new(),
                identity: None,
                session: SessionStatus::Missing,
                master_key_cached: None,
            });
        };
        let env_token = std::env::var("CLOUDSDK_AUTH_ACCESS_TOKEN").is_ok_and(|v| !v.is_empty());
        let creds = self.adc_credentials_path().is_some();
        let session = if env_token || creds || which_gcloud() {
            SessionStatus::Ambient
        } else {
            SessionStatus::Missing
        };
        Ok(BackendStatus {
            kind: "gcp",
            endpoint: format!("project {}", gcp.project),
            identity: None,
            session,
            master_key_cached: None,
        })
    }

    fn list(&mut self) -> Result<Vec<Entry>> {
        let mut out = Vec::new();
        let mut page_token: Option<String> = None;
        loop {
            let mut url = self.url("/secrets?pageSize=100")?;
            if let Some(t) = &page_token {
                url.push_str(&format!("&pageToken={t}"));
            }
            let resp = self
                .authed(self.agent.get(&url))?
                .call()
                .map_err(map_gcp_err)?;
            let body: ListSecretsResponse =
                resp.into_json().context("parsing list secrets response")?;
            for s in body.secrets {
                let annotations = s.annotations.unwrap_or_default();
                let updated_at = s
                    .create_time
                    .as_deref()
                    .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
                    .map(|d| d.with_timezone(&Utc));
                let labels = annotations
                    .get("confctl-labels")
                    .map(|v| v.split(',').map(str::to_string).collect::<Vec<_>>());
                out.push(Entry {
                    id: short_id(&s.name),
                    name: short_id(&s.name),
                    size: annotations
                        .get("confctl-size")
                        .and_then(|v| v.parse().ok()),
                    updated_at,
                    labels,
                    filename: annotations.get("confctl-filename").cloned(),
                });
            }
            page_token = body.next_page_token.filter(|t| !t.is_empty());
            if page_token.is_none() {
                break;
            }
        }
        Ok(out)
    }

    fn push(&mut self, req: PushRequest<'_>) -> Result<Entry> {
        let secret_id = sanitize_secret_id(&req.name);
        if secret_id != req.name {
            eprintln!("note: name {:?} adjusted to {secret_id:?} (Secret Manager allows only letters, digits, `-`, `_`).", req.name);
        }

        let mut annotations = std::collections::BTreeMap::new();
        if let Some(f) = &req.filename {
            annotations.insert("confctl-filename", f.clone());
        }
        annotations.insert(
            "confctl-mime",
            req.mime
                .clone()
                .unwrap_or_else(|| "application/octet-stream".into()),
        );
        annotations.insert("confctl-size", req.bytes.len().to_string());
        if !req.labels.is_empty() {
            annotations.insert("confctl-labels", req.labels.join(","));
        }

        let create_url = self.url(&format!("/secrets?secretId={secret_id}"))?;
        let create_body = json!({
            "replication": {"automatic": {}},
            "annotations": annotations,
        });
        let created = self
            .authed(self.agent.post(&create_url))?
            .send_json(&create_body);
        match created {
            Ok(_) => {}
            Err(ureq::Error::Status(409, _)) if req.overwrite => {
                // Exists — refresh annotations, then add a new version below.
                let patch_url = self.url(&format!(
                    "/secrets/{secret_id}?updateMask=annotations"
                ))?;
                self.authed(self.agent.request("PATCH", &patch_url))?
                    .send_json(json!({"annotations": annotations}))
                    .map_err(map_gcp_err)
                    .context("updating secret annotations")?;
            }
            Err(ureq::Error::Status(409, _)) => {
                bail!("a secret named {secret_id:?} already exists in project {}; pass --overwrite to add a new version", self.project()?)
            }
            Err(e) => return Err(map_gcp_err(e)).context("creating secret"),
        }

        let add_url = self.url(&format!("/secrets/{secret_id}:addVersion"))?;
        let resp = self
            .authed(self.agent.post(&add_url))?
            .send_json(json!({"payload": {"data": STANDARD.encode(req.bytes)}}))
            .map_err(map_gcp_err)
            .context("adding secret version")?;
        let version: serde_json::Value = resp.into_json().context("parsing addVersion response")?;
        let version_id = version
            .get("name")
            .and_then(|n| n.as_str())
            .map(short_id)
            .unwrap_or_default();

        Ok(Entry {
            id: format!("{secret_id} (version {version_id})"),
            name: secret_id,
            size: Some(req.bytes.len() as u64),
            updated_at: Some(Utc::now()),
            labels: None,
            filename: req.filename.clone(),
        })
    }

    fn pull(&mut self, req: PullRequest<'_>) -> Result<PulledSecret> {
        let name = req
            .name
            .or(req.id)
            .ok_or_else(|| anyhow!("provide a display name or --id"))?;
        let secret_id = sanitize_secret_id(name);

        let url = self.url(&format!("/secrets/{secret_id}/versions/latest:access"))?;
        let resp = self
            .authed(self.agent.get(&url))?
            .call()
            .map_err(map_gcp_err)?;
        let body: AccessVersionResponse = resp
            .into_json()
            .context("parsing access version response")?;
        let bytes = STANDARD
            .decode(&body.payload.data)
            .context("decoding base64 payload from Secret Manager")?;

        let filename = self
            .fetch_secret_meta(&secret_id)
            .ok()
            .and_then(|s| s.annotations.and_then(|a| a.get("confctl-filename").cloned()))
            .or(Some(secret_id));
        Ok(PulledSecret { bytes, filename })
    }

    fn rm(&mut self, name: &str) -> Result<()> {
        let secret_id = sanitize_secret_id(name);
        let url = self.url(&format!("/secrets/{secret_id}"))?;
        self.authed(self.agent.delete(&url))?
            .call()
            .map_err(map_gcp_err)?;
        Ok(())
    }
}

fn which_gcloud() -> bool {
    Command::new("gcloud")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// ---------- Error mapping ----------

fn map_gcp_err(err: ureq::Error) -> anyhow::Error {
    match err {
        ureq::Error::Status(status, resp) => {
            let body = resp
                .into_string()
                .unwrap_or_else(|_| format!("HTTP {status}"));
            // Google errors look like {"error": {"message": "...", "status": "..."}}.
            let pretty = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| {
                    v.get("error")
                        .and_then(|e| e.get("message"))
                        .and_then(|m| m.as_str())
                        .map(str::to_string)
                })
                .filter(|s| !s.is_empty())
                .unwrap_or(body);
            anyhow!("Google API returned {status}: {pretty}")
        }
        ureq::Error::Transport(t) => anyhow!("transport error against Google API: {t}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_folds_invalid_chars() {
        assert_eq!(sanitize_secret_id(".env"), "env");
        assert_eq!(sanitize_secret_id("my app/.env.prod"), "my-app-env-prod");
        assert_eq!(sanitize_secret_id("ok-name_123"), "ok-name_123");
    }

    #[test]
    fn sanitize_never_returns_empty() {
        assert_eq!(sanitize_secret_id("..."), "secret");
        assert_eq!(sanitize_secret_id(""), "secret");
    }

    #[test]
    fn short_id_strips_resource_prefix() {
        assert_eq!(short_id("projects/p1/secrets/my-env"), "my-env");
        assert_eq!(short_id("bare-name"), "bare-name");
    }
}
