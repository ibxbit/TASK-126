//! Startup cleanup for orphaned upload artifacts.
//!
//! Three classes of orphan can appear after a crash:
//!
//!   1. Staging directories under `<app_data>/staging/<session_uuid>/`
//!      whose session is no longer `in_progress` in SQLite. These are
//!      leftover chunk files from uploads that the user aborted or
//!      that completed but weren't cleaned up before the crash.
//!
//!   2. Staging directories whose name is a UUID but which have NO
//!      corresponding `upload_sessions` row at all. These are truly
//!      orphaned — the DB insert was rolled back but the directory
//!      creation already happened.
//!
//!   3. Stray `v_new.<session_uuid>.tmp` files inside attachment
//!      directories. Written during `finalize` and normally removed
//!      by `TmpGuard::drop`, but a process kill (power loss, task
//!      manager) bypasses Drop and leaves the tmp behind.
//!
//! `cleanup_orphaned_uploads` is called once per startup, AFTER the
//! general `recovery::run_at_startup` pass. Safe to call on every
//! startup (both clean and unclean) — it is a no-op when the on-disk
//! state already matches the DB.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Serialize;
use thiserror::Error;
use uuid::Uuid;

use crate::docs::storage::StorageLayout;

#[derive(Debug, Error, Serialize)]
#[serde(tag = "type", content = "detail", rename_all = "snake_case")]
pub enum CleanupError {
    #[error("persistence error: {0}")]
    Persistence(String),
    #[error("io error: {0}")]
    Io(String),
}

/// Counters describing what the cleanup did. Logged at startup and
/// surfaced to admin diagnostics.
#[derive(Debug, Default, Serialize, PartialEq, Eq)]
pub struct CleanupReport {
    /// Staging dirs removed because their session is no longer
    /// `in_progress` (finalized, aborted, or unknown).
    pub staging_dirs_removed: u32,
    /// Stray `v_new.*.tmp` files removed under the attachments root.
    pub tmp_files_removed: u32,
}

pub trait UploadRecoveryRepository {
    /// Ids of every `upload_sessions` row whose `status = 'in_progress'`.
    fn list_in_progress_session_ids(&self) -> Result<Vec<Uuid>, String>;
}

/// Cleanup pass. Idempotent and safe on every startup.
pub fn cleanup_orphaned_uploads<R: UploadRecoveryRepository>(
    repo: &R,
    layout: &StorageLayout,
) -> Result<CleanupReport, CleanupError> {
    let mut report = CleanupReport::default();

    // 1. Snapshot of sessions that MUST be preserved.
    let active: HashSet<Uuid> = repo
        .list_in_progress_session_ids()
        .map_err(CleanupError::Persistence)?
        .into_iter()
        .collect();

    // 2. Walk the staging root.
    let staging_root = layout.staging_root();
    if staging_root.exists() {
        let entries =
            std::fs::read_dir(&staging_root).map_err(|e| CleanupError::Io(e.to_string()))?;
        for entry in entries {
            let entry = entry.map_err(|e| CleanupError::Io(e.to_string()))?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = match path.file_name().and_then(|s| s.to_str()) {
                Some(n) => n,
                None => continue,
            };
            // Only manage directories whose name is a UUID — anything
            // else is NOT ours and must be left alone.
            let Ok(sid) = Uuid::parse_str(name) else {
                continue;
            };
            if active.contains(&sid) {
                continue;
            }
            // Orphan: remove best-effort. On Windows a file still
            // being written by a background thread would block
            // removal; the next sweep picks it up.
            if std::fs::remove_dir_all(&path).is_ok() {
                report.staging_dirs_removed += 1;
            }
        }
    }

    // 3. Walk the attachments root for stray finalize-tmp files.
    let attachments_root = layout.attachments_root();
    if attachments_root.exists() {
        let mut strays = Vec::new();
        collect_v_new_tmp(&attachments_root, &mut strays);
        for p in strays {
            if std::fs::remove_file(&p).is_ok() {
                report.tmp_files_removed += 1;
            }
        }
    }

    Ok(report)
}

/// Recursively collect any file named `v_new.<...>.tmp` under `dir`.
/// We match the specific prefix so an unrelated `.tmp` produced by
/// some other code path (e.g. `atomic_write` pairs that briefly
/// appear next to non-version files) is never touched here — those
/// are handled by the general recovery sweep.
fn collect_v_new_tmp(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_v_new_tmp(&p, out);
        } else if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
            if name.starts_with("v_new.") && name.ends_with(".tmp") {
                out.push(p);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use tempfile::tempdir;

    struct MockRepo {
        active: RefCell<Vec<Uuid>>,
    }
    impl UploadRecoveryRepository for MockRepo {
        fn list_in_progress_session_ids(&self) -> Result<Vec<Uuid>, String> {
            Ok(self.active.borrow().clone())
        }
    }

    fn make_staging_dir(layout: &StorageLayout, session_id: &Uuid, with_file: bool) -> PathBuf {
        let p = layout.staging_dir(session_id);
        std::fs::create_dir_all(&p).unwrap();
        if with_file {
            std::fs::write(p.join("chunk_000000"), b"data").unwrap();
        }
        p
    }

    fn make_v_new_tmp(layout: &StorageLayout, attachment_id: &Uuid) -> PathBuf {
        let tenant = Uuid::new_v4();
        let dir = layout.attachment_dir(&tenant, attachment_id, 1_700_000_000);
        std::fs::create_dir_all(&dir).unwrap();
        let tmp = dir.join(format!("v_new.{}.tmp", Uuid::new_v4()));
        std::fs::write(&tmp, b"partial").unwrap();
        tmp
    }

    #[test]
    fn removes_staging_dir_for_non_active_session() {
        let d = tempdir().unwrap();
        let layout = StorageLayout::new(d.path());
        let sid = Uuid::new_v4();
        let dir = make_staging_dir(&layout, &sid, true);
        assert!(dir.exists());

        let repo = MockRepo { active: RefCell::new(vec![]) };
        let r = cleanup_orphaned_uploads(&repo, &layout).unwrap();
        assert_eq!(r.staging_dirs_removed, 1);
        assert!(!dir.exists());
    }

    #[test]
    fn preserves_staging_dir_for_active_session() {
        let d = tempdir().unwrap();
        let layout = StorageLayout::new(d.path());
        let sid = Uuid::new_v4();
        let dir = make_staging_dir(&layout, &sid, true);

        let repo = MockRepo { active: RefCell::new(vec![sid]) };
        let r = cleanup_orphaned_uploads(&repo, &layout).unwrap();
        assert_eq!(r.staging_dirs_removed, 0);
        assert!(dir.exists(), "active session's staging dir must survive");
    }

    #[test]
    fn ignores_non_uuid_directories() {
        let d = tempdir().unwrap();
        let layout = StorageLayout::new(d.path());
        std::fs::create_dir_all(layout.staging_root().join("not-a-uuid")).unwrap();

        let repo = MockRepo { active: RefCell::new(vec![]) };
        let r = cleanup_orphaned_uploads(&repo, &layout).unwrap();
        assert_eq!(r.staging_dirs_removed, 0);
        assert!(layout.staging_root().join("not-a-uuid").exists());
    }

    #[test]
    fn removes_v_new_tmp_under_attachments() {
        let d = tempdir().unwrap();
        let layout = StorageLayout::new(d.path());
        let tmp = make_v_new_tmp(&layout, &Uuid::new_v4());
        assert!(tmp.exists());

        let repo = MockRepo { active: RefCell::new(vec![]) };
        let r = cleanup_orphaned_uploads(&repo, &layout).unwrap();
        assert_eq!(r.tmp_files_removed, 1);
        assert!(!tmp.exists());
    }

    #[test]
    fn does_not_touch_final_vN_bin() {
        let d = tempdir().unwrap();
        let layout = StorageLayout::new(d.path());
        let tenant = Uuid::new_v4();
        let att = Uuid::new_v4();
        let dir = layout.attachment_dir(&tenant, &att, 1_700_000_000);
        std::fs::create_dir_all(&dir).unwrap();
        let final_path = dir.join("v1.bin");
        std::fs::write(&final_path, b"real").unwrap();

        let repo = MockRepo { active: RefCell::new(vec![]) };
        let _ = cleanup_orphaned_uploads(&repo, &layout).unwrap();
        assert!(final_path.exists(), "v1.bin must survive cleanup");
    }

    #[test]
    fn noop_on_empty_filesystem() {
        let d = tempdir().unwrap();
        let layout = StorageLayout::new(d.path());
        let repo = MockRepo { active: RefCell::new(vec![]) };
        let r = cleanup_orphaned_uploads(&repo, &layout).unwrap();
        assert_eq!(r, CleanupReport::default());
    }

    #[test]
    fn idempotent_across_consecutive_runs() {
        let d = tempdir().unwrap();
        let layout = StorageLayout::new(d.path());
        let sid = Uuid::new_v4();
        make_staging_dir(&layout, &sid, true);
        make_v_new_tmp(&layout, &Uuid::new_v4());

        let repo = MockRepo { active: RefCell::new(vec![]) };
        let r1 = cleanup_orphaned_uploads(&repo, &layout).unwrap();
        assert!(r1.staging_dirs_removed + r1.tmp_files_removed > 0);

        let r2 = cleanup_orphaned_uploads(&repo, &layout).unwrap();
        assert_eq!(r2, CleanupReport::default());
    }
}
