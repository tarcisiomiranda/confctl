//! HTTP client for bunker-vault (sync, via ureq + rustls).
//!
//! Wire contract mirrors the Poem/OpenAPI types in
//! /srv/bunker-vault/crates/vault-server/src/api.rs:
//!
//!   POST /api/v1/auth/{salt,login,refresh,logout}
//!   GET  /api/v1/secrets
//!   POST /api/v1/secrets
//!   GET  /api/v1/secrets/:id
//!   PUT  /api/v1/secrets/:id
//!   DELETE /api/v1/secrets/:id
//!
//! Binary fields (`ciphertext`, `nonce`, `salt`, `master_key_hash`) are
//! base64 STANDARD strings; timestamps are ISO-8601 strings.

use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::crypto::SecretEnvelope;
use super::storage::AuthTokens;

const API_PREFIX: &str = "/api/v1";

// ---------- Wire types (match vault-server/src/api.rs) ----------

#[derive(Serialize)]
struct SaltRequestBody<'a> {
    email: &'a str,
}

#[derive(Deserialize)]
struct SaltResponseBody {
    salt: String,
}

#[derive(Serialize)]
struct LoginRequestBody<'a> {
    email: &'a str,
    master_key_hash: String,
}

#[derive(Serialize)]
struct RegisterRequestBody<'a> {
    email: &'a str,
    master_key_hash: String,
    kdf_salt: String,
}

#[derive(Deserialize)]
struct TokenResponseBody {
    token: String,
    refresh_token: String,
    expires_at: String,
}

#[derive(Serialize)]
struct RefreshRequestBody<'a> {
    refresh_token: &'a str,
}

#[derive(Serialize)]
struct LogoutRequestBody<'a> {
    refresh_token: &'a str,
}

#[derive(Serialize)]
struct SecretRequestBody<'a> {
    kind: &'a str,
    ciphertext: String,
    nonce: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SecretResponseDto {
    pub id: String,
    pub kind: String,
    pub ciphertext: String,
    pub nonce: String,
    pub created_at: String,
    pub updated_at: String,
}

impl SecretResponseDto {
    pub fn decode(&self) -> Result<(Vec<u8>, Vec<u8>)> {
        let ct = STANDARD
            .decode(&self.ciphertext)
            .context("decoding ciphertext from server")?;
        let nonce = STANDARD
            .decode(&self.nonce)
            .context("decoding nonce from server")?;
        Ok((ct, nonce))
    }
}

#[derive(Debug, Deserialize)]
struct SecretsListResponseBody {
    items: Vec<SecretResponseDto>,
    #[allow(dead_code)]
    total: usize,
}

#[derive(Debug, Deserialize)]
struct ErrorResponseBody {
    error: String,
    #[allow(dead_code)]
    code: String,
}

// ---------- Client ----------

pub struct HttpClient {
    base: String,
    agent: ureq::Agent,
}

impl HttpClient {
    pub fn new(base_url: &str) -> Self {
        let base = base_url.trim_end_matches('/').to_string();
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(60))
            .user_agent(concat!("confctl/", env!("CARGO_PKG_VERSION")))
            .build();
        Self { base, agent }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{API_PREFIX}{path}", self.base)
    }

    pub fn get_salt(&self, email: &str) -> Result<Vec<u8>> {
        let resp = self
            .agent
            .post(&self.url("/auth/salt"))
            .send_json(SaltRequestBody { email })
            .map_err(map_ureq_err)?;
        let body: SaltResponseBody = resp.into_json().context("parsing /auth/salt response")?;
        STANDARD
            .decode(&body.salt)
            .context("decoding salt from /auth/salt response")
    }

    /// Register a new user. Exposed for e2e tests — `confctl` itself does
    /// not surface account creation on the CLI.
    pub fn register(&self, email: &str, auth_hash: &[u8], kdf_salt: &[u8]) -> Result<AuthTokens> {
        let resp = self
            .agent
            .post(&self.url("/auth/register"))
            .send_json(RegisterRequestBody {
                email,
                master_key_hash: STANDARD.encode(auth_hash),
                kdf_salt: STANDARD.encode(kdf_salt),
            })
            .map_err(map_ureq_err)?;
        let body: TokenResponseBody = resp
            .into_json()
            .context("parsing /auth/register response")?;
        parse_token_response(body)
    }

    pub fn login(&self, email: &str, auth_hash: &[u8]) -> Result<AuthTokens> {
        let resp = self
            .agent
            .post(&self.url("/auth/login"))
            .send_json(LoginRequestBody {
                email,
                master_key_hash: STANDARD.encode(auth_hash),
            })
            .map_err(map_ureq_err)?;
        let body: TokenResponseBody = resp.into_json().context("parsing /auth/login response")?;
        parse_token_response(body)
    }

    pub fn refresh(&self, refresh_token: &str) -> Result<AuthTokens> {
        let resp = self
            .agent
            .post(&self.url("/auth/refresh"))
            .send_json(RefreshRequestBody { refresh_token })
            .map_err(map_ureq_err)?;
        let body: TokenResponseBody = resp.into_json().context("parsing /auth/refresh response")?;
        parse_token_response(body)
    }

    pub fn logout(&self, refresh_token: &str, access: &str) -> Result<()> {
        self.agent
            .post(&self.url("/auth/logout"))
            .set("Authorization", &format!("Bearer {access}"))
            .send_json(LogoutRequestBody { refresh_token })
            .map_err(map_ureq_err)?;
        Ok(())
    }

    pub fn list_secrets(&self, access: &str) -> Result<Vec<SecretResponseDto>> {
        let resp = self
            .agent
            .get(&self.url("/secrets"))
            .set("Authorization", &format!("Bearer {access}"))
            .call()
            .map_err(map_ureq_err)?;
        let body: SecretsListResponseBody =
            resp.into_json().context("parsing /secrets list response")?;
        Ok(body.items)
    }

    pub fn create_secret(
        &self,
        access: &str,
        kind: &str,
        env: &SecretEnvelope,
    ) -> Result<SecretResponseDto> {
        let resp = self
            .agent
            .post(&self.url("/secrets"))
            .set("Authorization", &format!("Bearer {access}"))
            .send_json(SecretRequestBody {
                kind,
                ciphertext: STANDARD.encode(&env.ciphertext),
                nonce: STANDARD.encode(env.nonce),
            })
            .map_err(map_ureq_err)?;
        resp.into_json().context("parsing POST /secrets response")
    }

    pub fn update_secret(
        &self,
        access: &str,
        id: &str,
        kind: &str,
        env: &SecretEnvelope,
    ) -> Result<SecretResponseDto> {
        let resp = self
            .agent
            .put(&self.url(&format!("/secrets/{id}")))
            .set("Authorization", &format!("Bearer {access}"))
            .send_json(SecretRequestBody {
                kind,
                ciphertext: STANDARD.encode(&env.ciphertext),
                nonce: STANDARD.encode(env.nonce),
            })
            .map_err(map_ureq_err)?;
        resp.into_json()
            .context("parsing PUT /secrets/:id response")
    }

    pub fn delete_secret(&self, access: &str, id: &str) -> Result<()> {
        self.agent
            .delete(&self.url(&format!("/secrets/{id}")))
            .set("Authorization", &format!("Bearer {access}"))
            .call()
            .map_err(map_ureq_err)?;
        Ok(())
    }
}

fn parse_token_response(body: TokenResponseBody) -> Result<AuthTokens> {
    let expires_at: DateTime<Utc> = DateTime::parse_from_rfc3339(&body.expires_at)
        .with_context(|| format!("parsing expires_at {:?}", body.expires_at))?
        .with_timezone(&Utc);
    Ok(AuthTokens {
        access: body.token,
        refresh: body.refresh_token,
        expires_at,
    })
}

/// 401 is surfaced as a distinct error so callers can try a silent refresh.
#[derive(Debug, thiserror::Error)]
#[error("unauthorized")]
pub struct Unauthorized;

fn map_ureq_err(err: ureq::Error) -> anyhow::Error {
    match err {
        ureq::Error::Status(status, resp) => {
            if status == 401 {
                return anyhow!(Unauthorized);
            }
            let body = resp
                .into_json::<ErrorResponseBody>()
                .map(|e| e.error)
                .unwrap_or_else(|_| format!("HTTP {status}"));
            anyhow!("vault server returned {status}: {body}")
        }
        ureq::Error::Transport(t) => anyhow!("transport error: {t}"),
    }
}

/// Helper: detect 401 anywhere in the error chain.
pub fn is_unauthorized(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| cause.is::<Unauthorized>())
}

// ---------- Integration test skeleton (manual, gated) ----------

#[cfg(test)]
mod tests {
    use super::*;

    /// Full roundtrip against a live bunker-vault.
    ///
    /// Register → login → create_secret → list_secrets → delete_secret.
    /// Gated by CONFCTL_E2E=1 so CI stays fast and offline.
    ///
    /// Run with a fresh local server:
    ///   JWT_SECRET=test DATABASE_URL="sqlite:///tmp/vault.db?mode=rwc" \
    ///     cargo run -p vault-cli -- serve --port 18080 --bind 127.0.0.1 &
    ///   CONFCTL_E2E=1 CONFCTL_VAULT_URL=http://127.0.0.1:18080 \
    ///     cargo test vault::client::tests::e2e_full_roundtrip -- --ignored
    #[test]
    #[ignore = "requires a running bunker-vault; run with CONFCTL_E2E=1"]
    fn e2e_full_roundtrip() {
        use crate::vault::crypto::{auth_hash, derive_master_key, encrypt};

        if std::env::var("CONFCTL_E2E").ok().as_deref() != Some("1") {
            return;
        }
        let url =
            std::env::var("CONFCTL_VAULT_URL").unwrap_or_else(|_| "http://127.0.0.1:18080".into());
        let client = HttpClient::new(&url);

        // Unique email per run to avoid 409 conflicts.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let email = format!("e2e-{nanos}@confctl.local");
        let password = b"e2e-pass-do-not-ship";
        let salt = {
            use rand::RngCore;
            let mut s = [0u8; 32];
            rand::rngs::OsRng.fill_bytes(&mut s);
            s
        };

        let master_key = derive_master_key(password, &salt).expect("derive");
        let hash = auth_hash(&master_key, &salt).expect("auth_hash");

        let tokens = client.register(&email, &hash, &salt).expect("register");

        // Re-login with same derived hash just to prove the salt/login path.
        let server_salt = client.get_salt(&email).expect("salt");
        assert_eq!(server_salt.as_slice(), &salt);
        let login_tokens = client.login(&email, &hash).expect("login");
        assert!(!login_tokens.access.is_empty());

        // Round-trip a secret.
        let plaintext = br#"{"kind":"file","data":{"filename":"t.env","mime_type":"text/plain","size":7,"content":"Rk9PPWJhcgo=","display_name":"t.env"}}"#;
        let env = encrypt(&master_key, plaintext).expect("encrypt");

        let created = client
            .create_secret(&tokens.access, "file", &env)
            .expect("create");
        assert_eq!(created.kind, "file");

        let items = client.list_secrets(&tokens.access).expect("list");
        assert!(items.iter().any(|i| i.id == created.id));

        client
            .delete_secret(&tokens.access, &created.id)
            .expect("delete");

        let items_after = client.list_secrets(&tokens.access).expect("list2");
        assert!(items_after.iter().all(|i| i.id != created.id));

        client
            .logout(&tokens.refresh, &tokens.access)
            .expect("logout");
    }

    #[test]
    fn url_join_trims_trailing_slash() {
        let c = HttpClient::new("http://localhost:8080/");
        assert_eq!(c.url("/secrets"), "http://localhost:8080/api/v1/secrets");

        let c = HttpClient::new("http://localhost:8080");
        assert_eq!(
            c.url("/auth/login"),
            "http://localhost:8080/api/v1/auth/login"
        );
    }
}
