//! Expiry enforcement.
//!
//! Two layers:
//!   1. `verify_access(now, record, password)` is called on every
//!      open attempt. It rejects expired or revoked packages BEFORE
//!      doing the password comparison, then verifies the Argon2
//!      hash. `Result::Ok` is the green light to decrypt the ZIP.
//!   2. `ExpirySweeper` is a background thread that wakes every
//!      `SWEEP_INTERVAL_SECS`, finds expired packages, deletes the
//!      on-disk artifact, and zeros the password hash so even a
//!      restored backup can't reanimate the share.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use argon2::password_hash::{PasswordHash, PasswordVerifier};
use argon2::Argon2;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::auth::{self, AuthError, Permission, Principal};

pub const DEFAULT_LIFETIME_SECONDS: i64 = 7 * 24 * 3600;
const SWEEP_INTERVAL_SECS: u64 = 5 * 60;

#[derive(Debug, Error, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExpiryError {
    #[error(transparent)]
    Auth(#[from] AuthError),

    #[error("share package not found: {0}")]
    NotFound(String),

    #[error("share package has expired")]
    Expired,

    #[error("share package was revoked")]
    Revoked,

    #[error("password did not match")]
    BadPassword,

    #[error("hash decode error: {0}")]
    Hash(String),

    #[error("persistence error: {0}")]
    Persistence(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageRecord {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub created_by: Uuid,
    /// Decrypted absolute path to the on-disk encrypted ZIP.
    pub artifact_path: PathBuf,
    pub password_hash: String,
    pub created_at_unix: i64,
    pub expires_at_unix: i64,
    pub revoked_at_unix: Option<i64>,
}

pub trait PackageRepository {
    fn load(&self, id: &Uuid) -> Result<Option<PackageRecord>, String>;

    /// All packages with `expires_at_unix <= now_unix` AND not yet
    /// scrubbed (artifact still on disk OR password_hash still set).
    fn list_expired(&self, now_unix: i64) -> Result<Vec<PackageRecord>, String>;

    /// Mark revoked + clear the password hash. Idempotent.
    fn mark_revoked(&self, id: &Uuid, now_unix: i64) -> Result<(), String>;

    /// Mark scrubbed (clear password_hash) after the artifact file is
    /// removed during the expiry sweep. Idempotent.
    fn mark_scrubbed(&self, id: &Uuid) -> Result<(), String>;

    fn record_access(&self, id: &Uuid, now_unix: i64) -> Result<(), String>;
}

/// Single access guard: timing-safe verification of an access
/// attempt. ALWAYS check expiry/revocation BEFORE password comparison
/// so we don't leak whether a password was correct on a dead package.
pub fn verify_access(
    record: &PackageRecord,
    password: &str,
    now_unix: i64,
) -> Result<(), ExpiryError> {
    if record.revoked_at_unix.is_some() {
        return Err(ExpiryError::Revoked);
    }
    if now_unix >= record.expires_at_unix {
        return Err(ExpiryError::Expired);
    }
    let parsed = PasswordHash::new(&record.password_hash)
        .map_err(|e| ExpiryError::Hash(e.to_string()))?;
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .map_err(|_| ExpiryError::BadPassword)
}

/// Manually revoke a package. Caller must hold `ExportReport` in the
/// package's tenant scope (same gate as creation).
pub fn revoke_package<R: PackageRepository>(
    repo: &R,
    principal: &Principal,
    package_id: Uuid,
    now_unix: i64,
) -> Result<(), ExpiryError> {
    let rec = repo
        .load(&package_id)
        .map_err(ExpiryError::Persistence)?
        .ok_or_else(|| ExpiryError::NotFound(package_id.to_string()))?;
    auth::require(principal, Permission::ExportReport, &rec.tenant_id)?;
    repo.mark_revoked(&package_id, now_unix)
        .map_err(ExpiryError::Persistence)
}

/// One pass: delete artifacts for every expired package and scrub
/// their password hashes. Returns the number of packages purged.
pub fn sweep_expired<R: PackageRepository>(
    repo: &R,
    now_unix: i64,
) -> Result<u32, ExpiryError> {
    let expired = repo
        .list_expired(now_unix)
        .map_err(ExpiryError::Persistence)?;
    let mut purged = 0u32;
    for rec in expired {
        // Best-effort delete; missing files are fine.
        let _ = std::fs::remove_file(&rec.artifact_path);
        repo.mark_scrubbed(&rec.id)
            .map_err(ExpiryError::Persistence)?;
        purged += 1;
    }
    Ok(purged)
}

/// Periodic sweeper. Wraps `sweep_expired` in a named thread.
pub struct ExpirySweeper {
    running: Arc<Mutex<bool>>,
}

impl Default for ExpirySweeper {
    fn default() -> Self {
        Self {
            running: Arc::new(Mutex::new(false)),
        }
    }
}

impl ExpirySweeper {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start<R>(&self, repo: Arc<R>) -> Option<thread::JoinHandle<()>>
    where
        R: PackageRepository + Send + Sync + 'static,
    {
        {
            let mut g = self.running.lock().expect("expiry sweeper poisoned");
            if *g {
                return None;
            }
            *g = true;
        }
        let running = Arc::clone(&self.running);
        let handle = thread::Builder::new()
            .name("shoreline-share-expiry".into())
            .spawn(move || loop {
                thread::sleep(Duration::from_secs(SWEEP_INTERVAL_SECS));
                if !*running.lock().expect("expiry sweeper poisoned") {
                    break;
                }
                let now = now_unix();
                let _ = sweep_expired(repo.as_ref(), now);
            })
            .expect("failed to spawn share expiry sweeper");
        Some(handle)
    }

    pub fn stop(&self) {
        let mut g = self.running.lock().expect("expiry sweeper poisoned");
        *g = false;
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use argon2::password_hash::SaltString;
    use argon2::PasswordHasher;
    use rand::rngs::OsRng;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use tempfile::tempdir;

    fn hashed(pw: &str) -> String {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(pw.as_bytes(), &salt)
            .unwrap()
            .to_string()
    }

    fn record(path: PathBuf, hash: String, expires: i64) -> PackageRecord {
        PackageRecord {
            id: Uuid::new_v4(),
            tenant_id: Uuid::new_v4(),
            created_by: Uuid::new_v4(),
            artifact_path: path,
            password_hash: hash,
            created_at_unix: 0,
            expires_at_unix: expires,
            revoked_at_unix: None,
        }
    }

    #[derive(Default)]
    struct MockRepo {
        rows: RefCell<HashMap<Uuid, PackageRecord>>,
    }
    impl PackageRepository for MockRepo {
        fn load(&self, id: &Uuid) -> Result<Option<PackageRecord>, String> {
            Ok(self.rows.borrow().get(id).cloned())
        }
        fn list_expired(&self, now: i64) -> Result<Vec<PackageRecord>, String> {
            Ok(self
                .rows
                .borrow()
                .values()
                .filter(|r| r.expires_at_unix <= now && !r.password_hash.is_empty())
                .cloned()
                .collect())
        }
        fn mark_revoked(&self, id: &Uuid, now: i64) -> Result<(), String> {
            if let Some(r) = self.rows.borrow_mut().get_mut(id) {
                r.revoked_at_unix = Some(now);
                r.password_hash.clear();
            }
            Ok(())
        }
        fn mark_scrubbed(&self, id: &Uuid) -> Result<(), String> {
            if let Some(r) = self.rows.borrow_mut().get_mut(id) {
                r.password_hash.clear();
            }
            Ok(())
        }
        fn record_access(&self, _: &Uuid, _: i64) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn correct_password_within_window_allows_access() {
        let r = record(PathBuf::new(), hashed("s3cret-pass"), 1000);
        assert!(verify_access(&r, "s3cret-pass", 500).is_ok());
    }

    #[test]
    fn expired_rejected_before_password_check() {
        // Even with the correct password, an expired record is denied.
        let r = record(PathBuf::new(), hashed("s3cret-pass"), 100);
        let err = verify_access(&r, "s3cret-pass", 200).unwrap_err();
        assert!(matches!(err, ExpiryError::Expired));
    }

    #[test]
    fn revoked_rejected_before_password_check() {
        let mut r = record(PathBuf::new(), hashed("s3cret-pass"), 1000);
        r.revoked_at_unix = Some(500);
        let err = verify_access(&r, "s3cret-pass", 600).unwrap_err();
        assert!(matches!(err, ExpiryError::Revoked));
    }

    #[test]
    fn wrong_password_rejected() {
        let r = record(PathBuf::new(), hashed("s3cret-pass"), 1000);
        let err = verify_access(&r, "wrong", 500).unwrap_err();
        assert!(matches!(err, ExpiryError::BadPassword));
    }

    #[test]
    fn sweep_deletes_artifact_and_clears_hash() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("expired.zip");
        std::fs::write(&path, b"data").unwrap();

        let rec = record(path.clone(), hashed("pw"), 100);
        let id = rec.id;
        let repo = MockRepo::default();
        repo.rows.borrow_mut().insert(id, rec);

        let purged = sweep_expired(&repo, 200).unwrap();
        assert_eq!(purged, 1);
        assert!(!path.exists());
        assert!(repo.rows.borrow().get(&id).unwrap().password_hash.is_empty());
    }

    #[test]
    fn sweep_skips_active_packages() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("active.zip");
        std::fs::write(&path, b"data").unwrap();

        let rec = record(path.clone(), hashed("pw"), 1_000_000);
        let repo = MockRepo::default();
        repo.rows.borrow_mut().insert(rec.id, rec);

        let purged = sweep_expired(&repo, 100).unwrap();
        assert_eq!(purged, 0);
        assert!(path.exists());
    }
}
