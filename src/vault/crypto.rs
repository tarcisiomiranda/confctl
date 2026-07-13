//! Client-side cryptography matching bunker-vault's vault-core.
//!
//! Parameters pinned bit-for-bit against
//! /srv/bunker-vault/crates/vault-core/src/crypto.rs — changing any of
//! them silently breaks auth or decryption. Parity is covered by the
//! frozen vectors in `extension/test-vectors/crypto-vectors.json`.

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    XChaCha20Poly1305, XNonce,
};
use thiserror::Error;
use zeroize::Zeroizing;

pub const KEY_LEN: usize = 32;
pub const NONCE_LEN: usize = 24;

pub const ARGON2_M_COST: u32 = 65536;
pub const ARGON2_T_COST: u32 = 3;
pub const ARGON2_P_COST: u32 = 4;

pub const AUTH_HASH_M_COST: u32 = 19456;
pub const AUTH_HASH_T_COST: u32 = 2;
pub const AUTH_HASH_P_COST: u32 = 1;

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("key derivation failed")]
    KeyDerivation,
    #[error("encryption failed")]
    Encryption,
    #[error("decryption failed (wrong key or corrupt ciphertext)")]
    Decryption,
    #[error("invalid nonce length: expected {NONCE_LEN}, got {0}")]
    InvalidNonce(usize),
}

pub type Result<T> = std::result::Result<T, CryptoError>;

pub struct SecretEnvelope {
    pub ciphertext: Vec<u8>,
    pub nonce: [u8; NONCE_LEN],
}

pub fn derive_master_key(password: &[u8], salt: &[u8]) -> Result<Zeroizing<[u8; KEY_LEN]>> {
    let params = Params::new(ARGON2_M_COST, ARGON2_T_COST, ARGON2_P_COST, Some(KEY_LEN))
        .map_err(|_| CryptoError::KeyDerivation)?;

    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);

    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    argon2
        .hash_password_into(password, salt, key.as_mut_slice())
        .map_err(|_| CryptoError::KeyDerivation)?;

    Ok(key)
}

pub fn auth_hash(master_key: &[u8; KEY_LEN], salt: &[u8]) -> Result<[u8; KEY_LEN]> {
    let params = Params::new(
        AUTH_HASH_M_COST,
        AUTH_HASH_T_COST,
        AUTH_HASH_P_COST,
        Some(KEY_LEN),
    )
    .map_err(|_| CryptoError::KeyDerivation)?;

    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);

    let mut out = [0u8; KEY_LEN];
    argon2
        .hash_password_into(master_key, salt, &mut out)
        .map_err(|_| CryptoError::KeyDerivation)?;

    Ok(out)
}

pub fn encrypt(key: &[u8; KEY_LEN], plaintext: &[u8]) -> Result<SecretEnvelope> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);

    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|_| CryptoError::Encryption)?;

    let mut nonce_bytes = [0u8; NONCE_LEN];
    nonce_bytes.copy_from_slice(nonce.as_slice());

    Ok(SecretEnvelope {
        ciphertext,
        nonce: nonce_bytes,
    })
}

pub fn decrypt(key: &[u8; KEY_LEN], ciphertext: &[u8], nonce: &[u8]) -> Result<Vec<u8>> {
    if nonce.len() != NONCE_LEN {
        return Err(CryptoError::InvalidNonce(nonce.len()));
    }

    let cipher = XChaCha20Poly1305::new(key.into());
    let nonce = XNonce::from_slice(nonce);

    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| CryptoError::Decryption)
}

#[cfg(test)]
pub(crate) fn encrypt_with_nonce(
    key: &[u8; KEY_LEN],
    nonce: &[u8; NONCE_LEN],
    plaintext: &[u8],
) -> Result<Vec<u8>> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let nonce = XNonce::from_slice(nonce);
    cipher
        .encrypt(nonce, plaintext)
        .map_err(|_| CryptoError::Encryption)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex_decode(s: &str) -> Vec<u8> {
        assert!(s.len().is_multiple_of(2));
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    fn hex_encode(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn key32(hex: &str) -> [u8; 32] {
        let bytes = hex_decode(hex);
        assert_eq!(bytes.len(), 32);
        let mut out = [0u8; 32];
        out.copy_from_slice(&bytes);
        out
    }

    fn nonce24(hex: &str) -> [u8; 24] {
        let bytes = hex_decode(hex);
        assert_eq!(bytes.len(), 24);
        let mut out = [0u8; 24];
        out.copy_from_slice(&bytes);
        out
    }

    // ---- Parity vectors copied verbatim from bunker-vault's
    // extension/test-vectors/crypto-vectors.json. Do not hand-edit these
    // constants — regenerate the file upstream and re-copy. ----

    #[test]
    fn parity_derive_key_short_password_zero_salt() {
        let password = hex_decode("70617373776f7264"); // "password"
        let salt = hex_decode("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f");
        let expected = "74ed3a2d9ce786bcff70336ea666b9acaee4cfc2ce826b28133b70b617983d52";

        let key = derive_master_key(&password, &salt).unwrap();
        assert_eq!(hex_encode(key.as_slice()), expected);
    }

    #[test]
    fn parity_derive_key_long_password_high_salt() {
        let password = hex_decode("636f727265637420686f727365206261747465727920737461706c65");
        let salt = hex_decode("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let expected = "76fffb9bfbb980fb5dd75acbd4cdf626a9ae2a0023390f3b317ebf0f85a6d917";

        let key = derive_master_key(&password, &salt).unwrap();
        assert_eq!(hex_encode(key.as_slice()), expected);
    }

    #[test]
    fn parity_auth_hash_roundtrip_from_derive_key() {
        let master_key = key32("74ed3a2d9ce786bcff70336ea666b9acaee4cfc2ce826b28133b70b617983d52");
        let salt = hex_decode("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f");
        let expected = "ad58d66570bcfaca8a83408d898a74015ba2ad1e0821e7605872869b636bfa5e";

        let hash = auth_hash(&master_key, &salt).unwrap();
        assert_eq!(hex_encode(&hash), expected);
    }

    #[test]
    fn parity_auth_hash_high_entropy() {
        let master_key = key32("1111111111111111111111111111111111111111111111111111111111111111");
        let salt = hex_decode("2222222222222222222222222222222222222222222222222222222222222222");
        let expected = "e28bf5e0b640d3e73405bc7000249fcbff4df847b87388419381ab41eec20716";

        let hash = auth_hash(&master_key, &salt).unwrap();
        assert_eq!(hex_encode(&hash), expected);
    }

    #[test]
    fn parity_aead_empty_plaintext() {
        let key = key32("3333333333333333333333333333333333333333333333333333333333333333");
        let nonce = nonce24("444444444444444444444444444444444444444444444444");
        let expected = "4ea82627a50618122afb7f7dc336f188";

        let ct = encrypt_with_nonce(&key, &nonce, b"").unwrap();
        assert_eq!(hex_encode(&ct), expected);

        let pt = decrypt(&key, &ct, &nonce).unwrap();
        assert!(pt.is_empty());
    }

    #[test]
    fn parity_aead_hello_world() {
        let key = key32("5555555555555555555555555555555555555555555555555555555555555555");
        let nonce = nonce24("666666666666666666666666666666666666666666666666");
        let plaintext = hex_decode("48656c6c6f2c2042756e6b6572205661756c7421");
        let expected = "ff4395c1197875b2b0831b8a38770b5845cc8d6b854f3fef4bd06c9acb4aa4ef4867576e";

        let ct = encrypt_with_nonce(&key, &nonce, &plaintext).unwrap();
        assert_eq!(hex_encode(&ct), expected);

        let pt = decrypt(&key, &ct, &nonce).unwrap();
        assert_eq!(pt, plaintext);
    }

    #[test]
    fn parity_aead_64_bytes_binary() {
        let key = key32("7777777777777777777777777777777777777777777777777777777777777777");
        let nonce = nonce24("888888888888888888888888888888888888888888888888");
        let plaintext = hex_decode(
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f\
             202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f",
        );
        let expected = "951cb02e6b57850a8adb41c4028f2ba7b47d693ad0a9d5819ed056ebe04d5375\
                        c6360d1b3d406f63c7cc617962ca4b7e0d3060a375a1141a787b274b61daf8c5\
                        e3ab8e1ffff00b65fb68345867ee48b2";

        let ct = encrypt_with_nonce(&key, &nonce, &plaintext).unwrap();
        assert_eq!(hex_encode(&ct), expected);

        let pt = decrypt(&key, &ct, &nonce).unwrap();
        assert_eq!(pt, plaintext);
    }

    #[test]
    fn random_nonce_roundtrip() {
        let key = key32("7777777777777777777777777777777777777777777777777777777777777777");
        let plaintext = b"hello world";

        let env = encrypt(&key, plaintext).unwrap();
        assert_eq!(env.nonce.len(), NONCE_LEN);
        assert_ne!(env.ciphertext.as_slice(), plaintext);

        let pt = decrypt(&key, &env.ciphertext, &env.nonce).unwrap();
        assert_eq!(pt, plaintext);
    }

    #[test]
    fn decrypt_wrong_nonce_length() {
        let key = key32("7777777777777777777777777777777777777777777777777777777777777777");
        let err = decrypt(&key, b"ciphertext", &[0u8; 16]).unwrap_err();
        assert!(matches!(err, CryptoError::InvalidNonce(16)));
    }

    #[test]
    fn decrypt_wrong_key_fails() {
        let k1 = key32("3333333333333333333333333333333333333333333333333333333333333333");
        let k2 = key32("4444444444444444444444444444444444444444444444444444444444444444");
        let env = encrypt(&k1, b"secret").unwrap();
        let err = decrypt(&k2, &env.ciphertext, &env.nonce).unwrap_err();
        assert!(matches!(err, CryptoError::Decryption));
    }
}
