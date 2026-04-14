//! Secure encryption-key management.
//!
//! Keys live in the OS credential store — Windows Credential Manager
//! on Windows (via the `keyring` crate). They are NEVER written to
//! SQLite and NEVER written to the filesystem. The in-memory
//! representation is zeroized on drop.
//!
//! The `rotation` submodule provides the safe, resumable migration
//! flow for re-encrypting every sensitive column when a new master
//! key is minted.
//!
//! Lifecycle:
//!   1. On first run, `get_or_create_master_key` generates a fresh
//!      32-byte key via the OS CSPRNG and persists it under the
//!      label "master_key.v1" for service "ShorelinePropertyOps".
//!   2. On subsequent startups, the same call retrieves the stored
//!      key from the OS store.
//!   3. Rotation is supported via `rotate_master_key` which writes a
//!      new version label (v2, v3, …); the caller re-encrypts data
//!      with the new key before `delete_master_key` retires the old
//!      label. (Rotation tooling lives outside this module.)
//!
//! The crate's DB field-level cipher (`db::encryption::FieldCipher`)
//! is constructed from the 32 bytes returned by this module — ONE
//! key, many AAD-scoped fields.

pub mod rotation;

pub use rotation::{
    default_specs, rotate_field, run_rotation, FieldOutcome, FieldSpec, KeyRotation,
    RotationError, RotationRepository, RotationSummary, RotationTx,
};

use std::sync::Mutex;

use base64::Engine;
use rand::rngs::OsRng;
use rand::RngCore;
use serde::Serialize;
use thiserror::Error;
use zeroize::Zeroize;

use crate::db::encryption::{CipherError, FieldCipher, KEY_LEN};

pub const KEYRING_SERVICE: &str = "ShorelinePropertyOps";
pub const LABEL_MASTER_V1: &str = "master_key.v1";

#[derive(Debug, Error, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum KeyError {
    #[error("keystore error: {0}")]
    Store(String),

    #[error("stored key is malformed (expected {expected} bytes, got {got})")]
    Malformed { expected: usize, got: usize },

    #[error("cipher error: {0}")]
    Cipher(String),
}

impl From<CipherError> for KeyError {
    fn from(e: CipherError) -> Self {
        KeyError::Cipher(e.to_string())
    }
}

// ── Store abstraction ───────────────────────────────────────────────────

/// Backend for persisting a secret by label. The Windows impl uses
/// Credential Manager; the in-memory impl is for tests.
pub trait KeyStore: Send + Sync {
    fn set(&self, label: &str, secret: &[u8]) -> Result<(), KeyError>;
    /// Returns `Ok(None)` when the label does not exist.
    fn get(&self, label: &str) -> Result<Option<Vec<u8>>, KeyError>;
    fn delete(&self, label: &str) -> Result<(), KeyError>;
}

// ── Windows Credential Manager (via keyring) ────────────────────────────

pub struct WindowsCredentialStore {
    service: String,
}

impl WindowsCredentialStore {
    pub fn new(service: impl Into<String>) -> Self {
        Self { service: service.into() }
    }

    pub fn default_service() -> Self {
        Self::new(KEYRING_SERVICE)
    }
}

impl KeyStore for WindowsCredentialStore {
    fn set(&self, label: &str, secret: &[u8]) -> Result<(), KeyError> {
        let entry = keyring::Entry::new(&self.service, label)
            .map_err(|e| KeyError::Store(e.to_string()))?;
        // keyring stores a text password; base64 keeps the key bytes
        // intact across the round-trip and makes the stored value
        // stable ASCII (friendly to Credential Manager's UTF-16 path).
        let encoded = base64::engine::general_purpose::STANDARD.encode(secret);
        entry
            .set_password(&encoded)
            .map_err(|e| KeyError::Store(e.to_string()))
    }

    fn get(&self, label: &str) -> Result<Option<Vec<u8>>, KeyError> {
        let entry = keyring::Entry::new(&self.service, label)
            .map_err(|e| KeyError::Store(e.to_string()))?;
        match entry.get_password() {
            Ok(encoded) => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(encoded.as_bytes())
                    .map_err(|e| KeyError::Store(e.to_string()))?;
                Ok(Some(bytes))
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(KeyError::Store(e.to_string())),
        }
    }

    fn delete(&self, label: &str) -> Result<(), KeyError> {
        let entry = keyring::Entry::new(&self.service, label)
            .map_err(|e| KeyError::Store(e.to_string()))?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()), // idempotent
            Err(e) => Err(KeyError::Store(e.to_string())),
        }
    }
}

// ── In-memory store (tests only) ────────────────────────────────────────

#[derive(Default)]
pub struct InMemoryKeyStore {
    inner: Mutex<std::collections::HashMap<String, Vec<u8>>>,
}

impl InMemoryKeyStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl KeyStore for InMemoryKeyStore {
    fn set(&self, label: &str, secret: &[u8]) -> Result<(), KeyError> {
        self.inner
            .lock()
            .map_err(|e| KeyError::Store(e.to_string()))?
            .insert(label.to_string(), secret.to_vec());
        Ok(())
    }
    fn get(&self, label: &str) -> Result<Option<Vec<u8>>, KeyError> {
        Ok(self
            .inner
            .lock()
            .map_err(|e| KeyError::Store(e.to_string()))?
            .get(label)
            .cloned())
    }
    fn delete(&self, label: &str) -> Result<(), KeyError> {
        self.inner
            .lock()
            .map_err(|e| KeyError::Store(e.to_string()))?
            .remove(label);
        Ok(())
    }
}

// ── MasterKey (zeroed on drop) ──────────────────────────────────────────

pub struct MasterKey {
    bytes: [u8; KEY_LEN],
}

impl MasterKey {
    pub fn generate() -> Self {
        let mut bytes = [0u8; KEY_LEN];
        OsRng.fill_bytes(&mut bytes);
        Self { bytes }
    }

    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Self { bytes }
    }

    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.bytes
    }

    /// Consume the master key to build the field cipher used by the
    /// DB encryption layer. After this call the key lives inside the
    /// cipher; the caller's `MasterKey` value has been moved.
    pub fn into_cipher(self) -> FieldCipher {
        FieldCipher::new(self.bytes)
    }
}

impl Drop for MasterKey {
    fn drop(&mut self) {
        self.bytes.zeroize();
    }
}

// ── KeyManager orchestration ────────────────────────────────────────────

pub struct KeyManager<S: KeyStore> {
    store: S,
}

impl<S: KeyStore> KeyManager<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }

    /// Fetch the master key, generating one on first run.
    ///
    /// - Startup path: `get(LABEL_MASTER_V1)` → `Some(bytes)` → verify
    ///   length → return.
    /// - First-run path: `get` returns `None` → `MasterKey::generate`
    ///   → `set` → return. Storage is atomic from the OS's
    ///   perspective, so concurrent processes that both generate a
    ///   key will end with one winning write; callers must treat this
    ///   call as a one-time initialization point (typically inside
    ///   the app bootstrap, before any DB opens).
    pub fn get_or_create_master_key(&self) -> Result<MasterKey, KeyError> {
        if let Some(bytes) = self.store.get(LABEL_MASTER_V1)? {
            if bytes.len() != KEY_LEN {
                return Err(KeyError::Malformed {
                    expected: KEY_LEN,
                    got: bytes.len(),
                });
            }
            let mut arr = [0u8; KEY_LEN];
            arr.copy_from_slice(&bytes);
            // Zero the intermediate Vec.
            let mut b = bytes;
            b.zeroize();
            return Ok(MasterKey::from_bytes(arr));
        }

        let fresh = MasterKey::generate();
        self.store.set(LABEL_MASTER_V1, fresh.as_bytes())?;
        Ok(fresh)
    }

    /// Write a new key under `new_label` without touching the
    /// previous one. Returns the new key so the caller can
    /// re-encrypt existing ciphertext before retiring the old label.
    pub fn rotate_master_key(&self, new_label: &str) -> Result<MasterKey, KeyError> {
        if self.store.get(new_label)?.is_some() {
            return Err(KeyError::Store(format!(
                "label '{new_label}' already exists — refusing to overwrite"
            )));
        }
        let fresh = MasterKey::generate();
        self.store.set(new_label, fresh.as_bytes())?;
        Ok(fresh)
    }

    /// Remove a key label from the OS store. Idempotent.
    pub fn delete_master_key(&self, label: &str) -> Result<(), KeyError> {
        self.store.delete(label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_run_generates_and_persists() {
        let store = InMemoryKeyStore::new();
        let mgr = KeyManager::new(store);
        let k1 = mgr.get_or_create_master_key().unwrap();
        assert_eq!(k1.as_bytes().len(), KEY_LEN);
    }

    #[test]
    fn second_call_returns_same_key() {
        let store = InMemoryKeyStore::new();
        let mgr = KeyManager::new(store);
        let k1 = *mgr.get_or_create_master_key().unwrap().as_bytes();
        let k2 = *mgr.get_or_create_master_key().unwrap().as_bytes();
        assert_eq!(k1, k2);
    }

    #[test]
    fn rotate_does_not_touch_existing_key() {
        let store = InMemoryKeyStore::new();
        let mgr = KeyManager::new(store);
        let k1 = *mgr.get_or_create_master_key().unwrap().as_bytes();
        let _k2 = mgr.rotate_master_key("master_key.v2").unwrap();
        let k1_again = *mgr.get_or_create_master_key().unwrap().as_bytes();
        assert_eq!(k1, k1_again);
    }

    #[test]
    fn rotate_refuses_to_overwrite() {
        let store = InMemoryKeyStore::new();
        let mgr = KeyManager::new(store);
        let _ = mgr.rotate_master_key("master_key.v2").unwrap();
        let err = mgr.rotate_master_key("master_key.v2").unwrap_err();
        assert!(matches!(err, KeyError::Store(_)));
    }

    #[test]
    fn delete_is_idempotent() {
        let store = InMemoryKeyStore::new();
        let mgr = KeyManager::new(store);
        mgr.delete_master_key(LABEL_MASTER_V1).unwrap();
        mgr.delete_master_key(LABEL_MASTER_V1).unwrap();
    }

    #[test]
    fn malformed_stored_key_is_rejected() {
        let store = InMemoryKeyStore::new();
        store.set(LABEL_MASTER_V1, &[0u8; 8]).unwrap(); // wrong length
        let mgr = KeyManager::new(store);
        let err = mgr.get_or_create_master_key().unwrap_err();
        assert!(matches!(err, KeyError::Malformed { .. }));
    }

    #[test]
    fn master_key_wires_into_field_cipher() {
        let store = InMemoryKeyStore::new();
        let mgr = KeyManager::new(store);
        let k = mgr.get_or_create_master_key().unwrap();
        let cipher = k.into_cipher();
        let aad = b"t.c:r";
        let ct = cipher.encrypt(b"hello", aad).unwrap();
        let pt = cipher.decrypt(&ct, aad).unwrap();
        assert_eq!(&pt, b"hello");
    }
}
