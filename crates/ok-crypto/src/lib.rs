//! Key hierarchy and sealed values (spec section 4).
use argon2::{Algorithm, Argon2, Params, Version};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use chacha20poly1305::{aead::{Aead, KeyInit, Payload}, XChaCha20Poly1305, XNonce};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::time::Instant;
use zeroize::{Zeroize, ZeroizeOnDrop};

pub const SEAL_PREFIX: &str = "xc20p:v1:";
pub const CANARY_PLAINTEXT: &[u8] = b"okremote-vault";

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("wrong passphrase or corrupted data")]
    Auth,
    #[error("invalid sealed value: {0}")]
    Format(String),
    #[error("unsupported key derivation: {0}")]
    Kdf(String),
}

/// 32-byte key that is wiped on drop.
#[derive(Clone, ZeroizeOnDrop)]
pub struct Key([u8; 32]);

impl Key {
    pub fn random() -> Self {
        let mut k = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut k);
        Key(k)
    }
    pub fn from_bytes(b: &[u8]) -> Result<Self, CryptoError> {
        let arr: [u8; 32] = b.try_into().map_err(|_| CryptoError::Format("key must be 32 bytes".into()))?;
        Ok(Key(arr))
    }
    pub fn as_bytes(&self) -> &[u8; 32] { &self.0 }
    pub fn to_base64(&self) -> String { B64.encode(self.0) }
    pub fn from_base64(s: &str) -> Result<Self, CryptoError> {
        let mut v = B64.decode(s.trim()).map_err(|e| CryptoError::Format(e.to_string()))?;
        let r = Self::from_bytes(&v);
        v.zeroize();
        r
    }
}

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("Key(..)") }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KdfParams {
    pub alg: String,
    pub m_cost: u32,
    pub t_cost: u32,
    pub p_cost: u32,
    pub salt: String,
}

impl KdfParams {
    pub fn new(t_cost: u32) -> Self {
        let mut salt = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut salt);
        KdfParams { alg: "argon2id".into(), m_cost: 65536, t_cost, p_cost: 4, salt: B64.encode(salt) }
    }
}

pub fn derive_kek(passphrase: &str, p: &KdfParams) -> Result<Key, CryptoError> {
    if p.alg != "argon2id" {
        return Err(CryptoError::Kdf(p.alg.clone()));
    }
    let salt = B64.decode(&p.salt).map_err(|e| CryptoError::Format(e.to_string()))?;
    let params = Params::new(p.m_cost, p.t_cost, p.p_cost, Some(32)).map_err(|e| CryptoError::Kdf(e.to_string()))?;
    let mut out = [0u8; 32];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(passphrase.as_bytes(), &salt, &mut out)
        .map_err(|e| CryptoError::Kdf(e.to_string()))?;
    let k = Key(out);
    out.zeroize();
    Ok(k)
}

/// Pick t_cost so one derivation takes ~300 ms here, clamped to 3..=12 (4.1).
pub fn calibrate() -> KdfParams {
    let probe = KdfParams::new(3);
    let start = Instant::now();
    let _ = derive_kek("calibration", &probe);
    let ms = start.elapsed().as_millis().max(1) as f64;
    let t = (3.0 * 300.0 / ms).round() as i64;
    KdfParams { t_cost: t.clamp(3, 12) as u32, ..probe }
}

pub fn aad(owner_id: &str, field: &str) -> Vec<u8> {
    let mut v = owner_id.as_bytes().to_vec();
    v.push(0);
    v.extend_from_slice(field.as_bytes());
    v
}

pub fn seal(key: &Key, plaintext: &[u8], aad: &[u8]) -> String {
    let cipher = XChaCha20Poly1305::new(key.as_bytes().into());
    let mut nonce = [0u8; 24];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let ct = cipher
        .encrypt(XNonce::from_slice(&nonce), Payload { msg: plaintext, aad })
        .expect("encryption cannot fail");
    let mut blob = nonce.to_vec();
    blob.extend_from_slice(&ct);
    format!("{SEAL_PREFIX}{}", B64.encode(blob))
}

pub fn open(key: &Key, sealed: &str, aad: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let rest = sealed.strip_prefix(SEAL_PREFIX).ok_or_else(|| CryptoError::Format("missing prefix".into()))?;
    let blob = B64.decode(rest).map_err(|e| CryptoError::Format(e.to_string()))?;
    if blob.len() < 24 + 16 {
        return Err(CryptoError::Format("too short".into()));
    }
    let (nonce, ct) = blob.split_at(24);
    XChaCha20Poly1305::new(key.as_bytes().into())
        .decrypt(XNonce::from_slice(nonce), Payload { msg: ct, aad })
        .map_err(|_| CryptoError::Auth)
}

pub fn random_token() -> String {
    let mut b = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_roundtrip_and_aad_binding() {
        let k = Key::random();
        let a = aad("owner-1", "secret");
        let s = seal(&k, b"hello", &a);
        assert!(s.starts_with(SEAL_PREFIX));
        assert_eq!(open(&k, &s, &a).unwrap(), b"hello");
        assert!(matches!(open(&k, &s, &aad("owner-2", "secret")), Err(CryptoError::Auth)));
        assert!(matches!(open(&k, &s, &aad("owner-1", "other")), Err(CryptoError::Auth)));
    }

    #[test]
    fn format_errors() {
        let k = Key::random();
        assert!(matches!(open(&k, "nope", b""), Err(CryptoError::Format(_))));
        assert!(matches!(open(&k, "xc20p:v1:!!!", b""), Err(CryptoError::Format(_))));
        assert!(matches!(open(&k, "xc20p:v1:AAAA", b""), Err(CryptoError::Format(_))));
    }

    #[test]
    fn kdf_deterministic_and_rejects_other_alg() {
        let mut p = KdfParams::new(1);
        p.m_cost = 1024;
        let a = derive_kek("pw", &p).unwrap();
        let b = derive_kek("pw", &p).unwrap();
        assert_eq!(a.as_bytes(), b.as_bytes());
        p.alg = "scrypt".into();
        assert!(derive_kek("pw", &p).is_err());
    }
}
