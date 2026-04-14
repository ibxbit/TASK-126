//! Field-level encryption for sensitive columns.
//!
//! Format of the BLOB persisted in SQLite:
//!
//!     ┌────────┬───────────────────────────────────────────────┐
//!     │ nonce  │ AES-256-GCM(ciphertext || auth_tag)           │
//!     │ 12 B   │                                               │
//!     └────────┴───────────────────────────────────────────────┘
//!
//! The master key is derived once per session from the operator's
//! unlock password (argon2id → 32 bytes). Callers MUST NOT reuse a
//! nonce for a given key; we mint a fresh random nonce for every
//! encryption, which is safe up to ~2^32 messages under GCM.
//!
//! Associated data (AAD) binds the ciphertext to its logical field
//! (e.g. "move_out_cases.notes_enc:<row_id>") so a ciphertext lifted
//! from one column cannot be pasted into another.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use zeroize::Zeroize;

pub const KEY_LEN: usize = 32;
pub const NONCE_LEN: usize = 12;

#[derive(Debug, Error, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CipherError {
    #[error("key length must be {expected} bytes, got {actual}")]
    BadKeyLength { expected: usize, actual: usize },
    #[error("ciphertext too short to contain a nonce")]
    Truncated,
    #[error("authenticated decryption failed — wrong key or tampered data")]
    Authentication,
    #[error("plaintext decoded as non-UTF8")]
    InvalidUtf8,
}

/// Wraps a 32-byte key, zeroed on drop.
pub struct FieldCipher {
    key: [u8; KEY_LEN],
}

impl Drop for FieldCipher {
    fn drop(&mut self) {
        self.key.zeroize();
    }
}

impl FieldCipher {
    pub fn new(key: [u8; KEY_LEN]) -> Self {
        Self { key }
    }

    pub fn from_slice(bytes: &[u8]) -> Result<Self, CipherError> {
        if bytes.len() != KEY_LEN {
            return Err(CipherError::BadKeyLength {
                expected: KEY_LEN,
                actual: bytes.len(),
            });
        }
        let mut key = [0u8; KEY_LEN];
        key.copy_from_slice(bytes);
        Ok(Self { key })
    }

    /// Encrypt `plaintext` binding it to `aad` (typically
    /// "<table>.<column>:<row_id>").
    pub fn encrypt(&self, plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>, CipherError> {
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&self.key));
        let mut nonce_bytes = [0u8; NONCE_LEN];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ct = cipher
            .encrypt(nonce, Payload { msg: plaintext, aad })
            .map_err(|_| CipherError::Authentication)?;

        let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
        out.extend_from_slice(&nonce_bytes);
        out.extend_from_slice(&ct);
        Ok(out)
    }

    pub fn decrypt(&self, blob: &[u8], aad: &[u8]) -> Result<Vec<u8>, CipherError> {
        if blob.len() < NONCE_LEN {
            return Err(CipherError::Truncated);
        }
        let (nonce_bytes, ct) = blob.split_at(NONCE_LEN);
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&self.key));
        cipher
            .decrypt(
                Nonce::from_slice(nonce_bytes),
                Payload { msg: ct, aad },
            )
            .map_err(|_| CipherError::Authentication)
    }

    pub fn encrypt_str(&self, plaintext: &str, aad: &[u8]) -> Result<Vec<u8>, CipherError> {
        self.encrypt(plaintext.as_bytes(), aad)
    }

    pub fn decrypt_str(&self, blob: &[u8], aad: &[u8]) -> Result<String, CipherError> {
        let bytes = self.decrypt(blob, aad)?;
        String::from_utf8(bytes).map_err(|_| CipherError::InvalidUtf8)
    }
}

/// Helper: build the canonical AAD for a (table, column, row id) triple.
pub fn aad_for(table: &str, column: &str, row_id: &str) -> Vec<u8> {
    format!("{table}.{column}:{row_id}").into_bytes()
}

/// Transparent marker type so service/repository signatures can clearly
/// distinguish "this Vec<u8> is an encrypted field blob, not arbitrary
/// bytes".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedField(pub Vec<u8>);

impl EncryptedField {
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> [u8; KEY_LEN] {
        [7u8; KEY_LEN]
    }

    #[test]
    fn round_trip() {
        let c = FieldCipher::new(test_key());
        let aad = aad_for("move_out_cases", "notes_enc", "row-1");
        let ct = c.encrypt_str("hello world", &aad).unwrap();
        assert_ne!(&ct[12..], b"hello world");
        let pt = c.decrypt_str(&ct, &aad).unwrap();
        assert_eq!(pt, "hello world");
    }

    #[test]
    fn aad_mismatch_rejected() {
        let c = FieldCipher::new(test_key());
        let aad1 = aad_for("move_out_cases", "notes_enc", "row-1");
        let aad2 = aad_for("move_out_cases", "notes_enc", "row-2");
        let ct = c.encrypt_str("secret", &aad1).unwrap();
        assert!(matches!(
            c.decrypt_str(&ct, &aad2),
            Err(CipherError::Authentication)
        ));
    }

    #[test]
    fn wrong_key_rejected() {
        let aad = aad_for("t", "c", "r");
        let ct = FieldCipher::new([1u8; 32]).encrypt_str("x", &aad).unwrap();
        assert!(matches!(
            FieldCipher::new([2u8; 32]).decrypt_str(&ct, &aad),
            Err(CipherError::Authentication)
        ));
    }

    #[test]
    fn truncated_blob_rejected() {
        let c = FieldCipher::new(test_key());
        assert!(matches!(c.decrypt(&[0u8; 4], b"aad"), Err(CipherError::Truncated)));
    }

    #[test]
    fn nonces_are_unique() {
        let c = FieldCipher::new(test_key());
        let aad = aad_for("t", "c", "r");
        let a = c.encrypt_str("same", &aad).unwrap();
        let b = c.encrypt_str("same", &aad).unwrap();
        assert_ne!(a[..12], b[..12]);
    }
}
