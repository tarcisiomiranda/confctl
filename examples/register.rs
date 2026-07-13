//! Developer tool: register a test account against a bunker-vault server.
//!
//! `confctl` itself does not expose account creation. This example exists so
//! end-to-end tests and local demos have a way to seed an account without
//! reaching into the vault's own CLI. Parameters match bunker-vault's
//! vault-core crypto bit-for-bit.
//!
//! Usage:
//!   cargo run --example register -- <url> <email> <password>

use argon2::{Algorithm, Argon2, Params, Version};
use base64::{engine::general_purpose::STANDARD, Engine};
use rand::RngCore;
use serde::Serialize;

const KEY_LEN: usize = 32;
const ARGON2_M: u32 = 65536;
const ARGON2_T: u32 = 3;
const ARGON2_P: u32 = 4;
const AUTH_M: u32 = 19456;
const AUTH_T: u32 = 2;
const AUTH_P: u32 = 1;

fn argon2_into(
    m: u32,
    t: u32,
    p: u32,
    password: &[u8],
    salt: &[u8],
    out: &mut [u8; KEY_LEN],
) -> anyhow::Result<()> {
    let params = Params::new(m, t, p, Some(KEY_LEN)).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    argon2
        .hash_password_into(password, salt, out)
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    Ok(())
}

#[derive(Serialize)]
struct RegisterBody<'a> {
    email: &'a str,
    master_key_hash: String,
    kdf_salt: String,
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("usage: {} <url> <email> <password>", args[0]);
        std::process::exit(2);
    }
    let url = args[1].trim_end_matches('/');
    let email = args[2].as_str();
    let password = args[3].as_bytes();

    let mut salt = [0u8; KEY_LEN];
    rand::rngs::OsRng.fill_bytes(&mut salt);

    let mut master_key = [0u8; KEY_LEN];
    argon2_into(
        ARGON2_M,
        ARGON2_T,
        ARGON2_P,
        password,
        &salt,
        &mut master_key,
    )?;

    let mut hash = [0u8; KEY_LEN];
    argon2_into(AUTH_M, AUTH_T, AUTH_P, &master_key, &salt, &mut hash)?;

    let body = RegisterBody {
        email,
        master_key_hash: STANDARD.encode(hash),
        kdf_salt: STANDARD.encode(salt),
    };

    let endpoint = format!("{url}/api/v1/auth/register");
    let resp = ureq::post(&endpoint).send_json(serde_json::to_value(&body)?);
    match resp {
        Ok(r) => {
            let txt = r.into_string().unwrap_or_default();
            println!("registered {email} against {url}: {txt}");
            Ok(())
        }
        Err(ureq::Error::Status(s, r)) => {
            let txt = r.into_string().unwrap_or_default();
            anyhow::bail!("register failed with HTTP {s}: {txt}")
        }
        Err(e) => anyhow::bail!("register transport error: {e}"),
    }
}
