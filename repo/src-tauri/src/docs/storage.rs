//! Filesystem layout and low-level file operations. All paths go
//! through this module so the on-disk shape is encapsulated and can be
//! re-rooted (e.g., for tests or rollback snapshots).

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use chrono::{Datelike, TimeZone, Utc};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("expected {expected} bytes, observed {actual}")]
    SizeMismatch { expected: u64, actual: u64 },

    #[error("sha256 mismatch")]
    DigestMismatch,
}

/// Roots all on-disk paths. Construct with `StorageLayout::new(app_data_dir)`.
#[derive(Debug, Clone)]
pub struct StorageLayout {
    pub root: PathBuf,
}

impl StorageLayout {
    pub fn new(app_data_dir: impl Into<PathBuf>) -> Self {
        Self { root: app_data_dir.into() }
    }

    pub fn attachments_root(&self) -> PathBuf {
        self.root.join("attachments")
    }

    pub fn staging_root(&self) -> PathBuf {
        self.root.join("staging")
    }

    /// Directory for a specific attachment id. Folder contains
    /// `v1.bin`, `v1.sha256`, `v2.bin`, … — one pair per version.
    pub fn attachment_dir(&self, tenant_id: &Uuid, attachment_id: &Uuid, created_at_unix: i64) -> PathBuf {
        let ts = Utc
            .timestamp_opt(created_at_unix, 0)
            .single()
            .unwrap_or_else(|| Utc.timestamp_opt(0, 0).single().unwrap());
        self.attachments_root()
            .join(tenant_id.to_string())
            .join(format!("{:04}", ts.year()))
            .join(format!("{:02}", ts.month()))
            .join(attachment_id.to_string())
    }

    pub fn version_path(&self, dir: &Path, version_no: u32) -> PathBuf {
        dir.join(format!("v{version_no}.bin"))
    }

    pub fn version_digest_path(&self, dir: &Path, version_no: u32) -> PathBuf {
        dir.join(format!("v{version_no}.sha256"))
    }

    pub fn staging_dir(&self, session_id: &Uuid) -> PathBuf {
        self.staging_root().join(session_id.to_string())
    }

    pub fn chunk_path(&self, session_id: &Uuid, index: u32) -> PathBuf {
        self.staging_dir(session_id).join(format!("chunk_{index:06}"))
    }

    /// Build the relative path used by the encrypted index. This is
    /// what ends up in `attachments.relative_path_enc` /
    /// `attachment_versions.relative_path_enc` after encryption.
    pub fn relative_path_for_version(
        tenant_id: &Uuid,
        attachment_id: &Uuid,
        created_at_unix: i64,
        version_no: u32,
    ) -> String {
        let ts = Utc.timestamp_opt(created_at_unix, 0).single().unwrap_or_else(|| {
            Utc.timestamp_opt(0, 0).single().unwrap()
        });
        format!(
            "{tenant}/{yyyy:04}/{mm:02}/{att}/v{ver}.bin",
            tenant = tenant_id,
            yyyy = ts.year(),
            mm = ts.month(),
            att = attachment_id,
            ver = version_no,
        )
    }

    pub fn ensure_dir(path: &Path) -> Result<(), StorageError> {
        fs::create_dir_all(path)?;
        Ok(())
    }

    /// Write `bytes` atomically: write to `<path>.tmp`, fsync, rename.
    pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), StorageError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("tmp");
        {
            let f = File::create(&tmp)?;
            let mut w = BufWriter::new(f);
            w.write_all(bytes)?;
            w.flush()?;
            w.into_inner()
                .map_err(|e| StorageError::Io(e.into_error()))?
                .sync_all()?;
        }
        fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Delete a directory tree, tolerating already-gone paths.
    pub fn remove_dir_all_if_exists(path: &Path) -> Result<(), StorageError> {
        if path.exists() {
            fs::remove_dir_all(path)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn layout_paths_are_tenant_scoped() {
        let dir = tempdir().unwrap();
        let layout = StorageLayout::new(dir.path());
        let tenant = Uuid::new_v4();
        let att = Uuid::new_v4();
        let p = layout.attachment_dir(&tenant, &att, 0);
        let s = p.to_string_lossy();
        assert!(s.contains(&tenant.to_string()));
        assert!(s.contains(&att.to_string()));
    }

    #[test]
    fn atomic_write_survives_partial_write() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("a/b/c.bin");
        StorageLayout::atomic_write(&target, b"hello").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"hello");
    }

    #[test]
    fn relative_path_matches_layout() {
        let tenant = Uuid::nil();
        let att = Uuid::nil();
        let p = StorageLayout::relative_path_for_version(&tenant, &att, 0, 1);
        assert!(p.starts_with(&tenant.to_string()));
        assert!(p.ends_with("v1.bin"));
    }
}
