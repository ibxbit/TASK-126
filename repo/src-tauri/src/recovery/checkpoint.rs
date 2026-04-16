//! Checkpoint-based recovery.
//!
//! Sequence at startup (single blocking pass before the UI is shown):
//!   1. Look for `lockfile` under the app data dir.
//!   2. Absent  → "clean_start": just write a fresh lockfile.
//!   3. Present → "unclean_repaired" path:
//!        a. WAL checkpoint(TRUNCATE) — flushes the WAL fully into
//!           the main DB file (no half-written pages).
//!        b. PRAGMA integrity_check — if it returns anything other
//!           than "ok", record `integrity_failed`; the caller may
//!           offer rollback.
//!        c. Sweep configured directories for stray `*.tmp` files
//!           (atomic-write residue) and remove them.
//!        d. Write a fresh lockfile.
//!
//! All disk operations are encapsulated behind the `StartupOps` trait
//! so they can be mocked. Real impls live behind the SQLite + fs
//! adapters.

use std::path::{Path, PathBuf};

use serde::Serialize;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error, Serialize)]
#[serde(tag = "type", content = "detail", rename_all = "snake_case")]
pub enum RecoveryError {
    #[error("io error: {0}")]
    Io(String),
    #[error("integrity check failed: {0}")]
    Integrity(String),
    #[error("persistence error: {0}")]
    Persistence(String),
}

impl From<std::io::Error> for RecoveryError {
    fn from(e: std::io::Error) -> Self {
        RecoveryError::Io(e.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryOutcome {
    CleanStart,
    UncleanRepaired,
    IntegrityFailed,
}

impl RecoveryOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            RecoveryOutcome::CleanStart => "clean_start",
            RecoveryOutcome::UncleanRepaired => "unclean_repaired",
            RecoveryOutcome::IntegrityFailed => "integrity_failed",
        }
    }
}

/// Operations the recovery manager needs at startup. The real impl
/// is the SQLite repository wrapped with filesystem helpers.
pub trait StartupOps {
    fn wal_checkpoint_truncate(&self) -> Result<(), String>;
    /// Returns SQLite's response, expected to be exactly "ok" when
    /// the database is healthy.
    fn integrity_check(&self) -> Result<String, String>;
    /// Returns the absolute paths of `*.tmp` files removed.
    fn sweep_orphan_tmp_files(&self, roots: &[PathBuf]) -> Result<Vec<PathBuf>, String>;
}

pub trait RecoveryRepository {
    fn record_event(
        &self,
        outcome: RecoveryOutcome,
        started_at_unix: i64,
        completed_at_unix: i64,
        details: &str,
    ) -> Result<(), String>;
}

const LOCKFILE_NAME: &str = "lockfile";

pub fn lockfile_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(LOCKFILE_NAME)
}

/// Write/refresh the per-process lockfile. Should be called once on
/// each successful startup AFTER recovery completes.
pub fn write_lockfile(app_data_dir: &Path) -> Result<(), RecoveryError> {
    std::fs::create_dir_all(app_data_dir)?;
    let pid = std::process::id();
    let now = now_unix();
    let body = format!("{pid}\n{now}\n");
    let path = lockfile_path(app_data_dir);
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// Best-effort lockfile removal — call from the graceful-shutdown hook.
pub fn remove_lockfile(app_data_dir: &Path) {
    let _ = std::fs::remove_file(lockfile_path(app_data_dir));
}

/// Run the full recovery sequence. Returns the observed outcome and
/// records a row in `recovery_events`.
pub fn run_at_startup<O: StartupOps, R: RecoveryRepository>(
    ops: &O,
    repo: &R,
    app_data_dir: &Path,
    sweep_roots: &[PathBuf],
) -> Result<RecoveryOutcome, RecoveryError> {
    let started = now_unix();
    let lock = lockfile_path(app_data_dir);

    let outcome = if !lock.exists() {
        // First start, or previous shutdown was clean.
        write_lockfile(app_data_dir)?;
        RecoveryOutcome::CleanStart
    } else {
        // Unclean shutdown — repair pass.
        ops.wal_checkpoint_truncate()
            .map_err(RecoveryError::Persistence)?;

        let integrity = ops.integrity_check().map_err(RecoveryError::Persistence)?;
        if integrity.trim() != "ok" {
            // Don't write a fresh lockfile — leave the marker so the
            // operator knows recovery failed. The UI can then surface
            // a rollback prompt.
            let details = format!("integrity_check returned: {integrity}");
            let _ = repo.record_event(
                RecoveryOutcome::IntegrityFailed,
                started,
                now_unix(),
                &details,
            );
            return Err(RecoveryError::Integrity(integrity));
        }

        let removed = ops
            .sweep_orphan_tmp_files(sweep_roots)
            .map_err(RecoveryError::Persistence)?;
        write_lockfile(app_data_dir)?;
        let details = format!("removed {} stray .tmp file(s)", removed.len());
        let _ = repo.record_event(
            RecoveryOutcome::UncleanRepaired,
            started,
            now_unix(),
            &details,
        );
        return Ok(RecoveryOutcome::UncleanRepaired);
    };

    let _ = repo.record_event(outcome, started, now_unix(), "");
    Ok(outcome)
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Convenience: scan `roots` recursively (one level for cheapness)
/// and return any file ending in `.tmp`. Used by SQLite-backed
/// `StartupOps` implementations.
pub fn find_tmp_files_recursive(roots: &[PathBuf]) -> std::io::Result<Vec<PathBuf>> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out)?;
            } else if path.extension().and_then(|s| s.to_str()) == Some("tmp") {
                out.push(path);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    for r in roots {
        if r.exists() {
            walk(r, &mut out)?;
        }
    }
    Ok(out)
}

/// Generate a stable id for a recovery event. Pure helper so the
/// SQLite repository doesn't need to hand-roll uuids.
pub fn new_event_id() -> Uuid {
    Uuid::new_v4()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use tempfile::tempdir;

    #[derive(Default)]
    struct MockOps {
        integrity: RefCell<String>,
        wal_called: RefCell<bool>,
        swept: RefCell<Vec<PathBuf>>,
    }
    impl StartupOps for MockOps {
        fn wal_checkpoint_truncate(&self) -> Result<(), String> {
            *self.wal_called.borrow_mut() = true;
            Ok(())
        }
        fn integrity_check(&self) -> Result<String, String> {
            Ok(self.integrity.borrow().clone())
        }
        fn sweep_orphan_tmp_files(&self, roots: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
            let mut all = Vec::new();
            for r in roots {
                if let Ok(found) = find_tmp_files_recursive(&[r.clone()]) {
                    all.extend(found.clone());
                    for p in found {
                        let _ = std::fs::remove_file(&p);
                    }
                }
            }
            *self.swept.borrow_mut() = all.clone();
            Ok(all)
        }
    }

    #[derive(Default)]
    struct MockRepo {
        events: RefCell<Vec<(RecoveryOutcome, String)>>,
    }
    impl RecoveryRepository for MockRepo {
        fn record_event(
            &self,
            outcome: RecoveryOutcome,
            _s: i64,
            _e: i64,
            details: &str,
        ) -> Result<(), String> {
            self.events.borrow_mut().push((outcome, details.into()));
            Ok(())
        }
    }

    #[test]
    fn first_start_is_clean() {
        let dir = tempdir().unwrap();
        let ops = MockOps::default();
        *ops.integrity.borrow_mut() = "ok".into();
        let repo = MockRepo::default();
        let outcome = run_at_startup(&ops, &repo, dir.path(), &[]).unwrap();
        assert_eq!(outcome, RecoveryOutcome::CleanStart);
        assert!(*ops.wal_called.borrow() == false);
        assert!(lockfile_path(dir.path()).exists());
        assert_eq!(repo.events.borrow()[0].0, RecoveryOutcome::CleanStart);
    }

    #[test]
    fn lingering_lockfile_triggers_repair() {
        let dir = tempdir().unwrap();
        std::fs::write(lockfile_path(dir.path()), "1234\n0\n").unwrap();
        let ops = MockOps::default();
        *ops.integrity.borrow_mut() = "ok".into();
        let repo = MockRepo::default();
        let outcome = run_at_startup(&ops, &repo, dir.path(), &[]).unwrap();
        assert_eq!(outcome, RecoveryOutcome::UncleanRepaired);
        assert!(*ops.wal_called.borrow());
        assert!(lockfile_path(dir.path()).exists()); // refreshed
    }

    #[test]
    fn integrity_failure_surfaces_and_keeps_lockfile() {
        let dir = tempdir().unwrap();
        let lock = lockfile_path(dir.path());
        std::fs::write(&lock, "1234\n0\n").unwrap();
        let ops = MockOps::default();
        *ops.integrity.borrow_mut() = "*** in database disk image is malformed".into();
        let repo = MockRepo::default();
        let err = run_at_startup(&ops, &repo, dir.path(), &[]).unwrap_err();
        assert!(matches!(err, RecoveryError::Integrity(_)));
        // Lockfile is intentionally left in place so the UI can detect
        // the failure and offer rollback.
        assert!(lock.exists());
        assert_eq!(repo.events.borrow()[0].0, RecoveryOutcome::IntegrityFailed);
    }

    #[test]
    fn graceful_shutdown_removes_lockfile() {
        let dir = tempdir().unwrap();
        write_lockfile(dir.path()).unwrap();
        assert!(lockfile_path(dir.path()).exists());
        remove_lockfile(dir.path());
        assert!(!lockfile_path(dir.path()).exists());
    }

    #[test]
    fn tmp_sweep_finds_residue() {
        let dir = tempdir().unwrap();
        let nested = dir.path().join("attachments").join("a");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("good.bin"), b"x").unwrap();
        std::fs::write(nested.join("half.tmp"), b"x").unwrap();
        let found = find_tmp_files_recursive(&[dir.path().to_path_buf()]).unwrap();
        assert_eq!(found.len(), 1);
        assert!(found[0].ends_with("half.tmp"));
    }
}
