//! Rollback to the previously-installed version.
//!
//! Sequence:
//!   1. Locate the previous (`is_active = 0`) row with the most recent
//!      `installed_at` and verify its `snapshot_path` exists on disk.
//!   2. Stop services / release file handles (caller responsibility;
//!      we accept a `RollbackOps::quiesce` callback).
//!   3. Restore the snapshot back into the live install location
//!      (binaries + DB).
//!   4. Flip `is_active` back to the previous version in a single
//!      transaction. The rolled-from version is retained as the new
//!      "previous" so a second rollback would only undo a future
//!      install — not double-rollback into nothingness.
//!   5. Restart hint returned.

use std::path::PathBuf;

use serde::Serialize;
use thiserror::Error;
use uuid::Uuid;

use crate::auth::{self, AuthError, Permission, Principal};
use crate::update::installer::{InstalledVersion, VersionRepository};

#[derive(Debug, Error, Serialize)]
#[serde(tag = "type", content = "detail", rename_all = "snake_case")]
pub enum RollbackError {
    #[error(transparent)]
    Auth(#[from] AuthError),

    #[error("no previous version available to roll back to")]
    NoPrevious,

    #[error("snapshot directory missing or unreadable: {0}")]
    MissingSnapshot(String),

    #[error("restore failed: {0}")]
    Restore(String),

    #[error("activation failed: {0}")]
    Activation(String),

    #[error("persistence error: {0}")]
    Persistence(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct RollbackOutcome {
    pub from_version: String,
    pub to_version: String,
    pub restart_required: bool,
}

pub trait RollbackOps {
    /// Caller releases file handles, closes the DB, etc. Best-effort.
    fn quiesce(&self) -> Result<(), String>;
    /// Copy the contents of `snapshot` over the live install location.
    fn restore_from_snapshot(&self, snapshot: &PathBuf) -> Result<(), String>;
}

pub trait RollbackRepository: VersionRepository {
    /// Make `target_version_id` active and the currently-active row
    /// inactive — single transaction.
    fn activate_version(&self, target_version_id: &Uuid) -> Result<(), String>;
}

pub fn rollback_to_previous<R: RollbackRepository, O: RollbackOps>(
    repo: &R,
    ops: &O,
    principal: &Principal,
    tenant_for_audit: Uuid,
) -> Result<RollbackOutcome, RollbackError> {
    auth::require(principal, Permission::ConfigurePermissions, &tenant_for_audit)?;

    let current = repo
        .active()
        .map_err(RollbackError::Persistence)?
        .ok_or(RollbackError::NoPrevious)?;
    let previous = repo
        .previous()
        .map_err(RollbackError::Persistence)?
        .ok_or(RollbackError::NoPrevious)?;

    let snapshot_path = previous
        .snapshot_path
        .clone()
        .ok_or_else(|| RollbackError::MissingSnapshot("none recorded".into()))?;
    if !snapshot_path.exists() {
        return Err(RollbackError::MissingSnapshot(
            snapshot_path.display().to_string(),
        ));
    }

    ops.quiesce().map_err(RollbackError::Restore)?;
    ops.restore_from_snapshot(&snapshot_path)
        .map_err(RollbackError::Restore)?;

    repo.activate_version(&previous.id)
        .map_err(RollbackError::Activation)?;

    Ok(RollbackOutcome {
        from_version: current.version,
        to_version: previous.version,
        restart_required: true,
    })
}

/// Helper for tests + concrete repos: derive the "previous" row from a
/// list of installed versions (most-recent inactive row wins).
pub fn derive_previous(rows: &[InstalledVersion]) -> Option<InstalledVersion> {
    let mut inactive: Vec<&InstalledVersion> = rows.iter().filter(|r| !r.is_active).collect();
    inactive.sort_by_key(|r| -r.installed_at_unix);
    inactive.first().map(|&v| v.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::context::TenantScope;
    use crate::auth::Role;
    use std::cell::RefCell;
    use tempfile::tempdir;

    struct MockRepo {
        active: RefCell<Option<InstalledVersion>>,
        rows: RefCell<Vec<InstalledVersion>>,
    }
    impl VersionRepository for MockRepo {
        fn active(&self) -> Result<Option<InstalledVersion>, String> {
            Ok(self.active.borrow().clone())
        }
        fn previous(&self) -> Result<Option<InstalledVersion>, String> {
            Ok(derive_previous(&self.rows.borrow()))
        }
        fn exists(&self, _: &str) -> Result<bool, String> { Ok(false) }
        fn install_and_activate(&self, _: &InstalledVersion) -> Result<(), String> { Ok(()) }
        fn prune_older_than_previous(&self) -> Result<Vec<PathBuf>, String> { Ok(vec![]) }
    }
    impl RollbackRepository for MockRepo {
        fn activate_version(&self, target: &Uuid) -> Result<(), String> {
            for r in self.rows.borrow_mut().iter_mut() {
                r.is_active = &r.id == target;
            }
            *self.active.borrow_mut() = self
                .rows
                .borrow()
                .iter()
                .find(|r| r.is_active)
                .cloned();
            Ok(())
        }
    }

    #[derive(Default)]
    struct MockOps {
        restored_from: RefCell<Option<PathBuf>>,
    }
    impl RollbackOps for MockOps {
        fn quiesce(&self) -> Result<(), String> { Ok(()) }
        fn restore_from_snapshot(&self, snapshot: &PathBuf) -> Result<(), String> {
            *self.restored_from.borrow_mut() = Some(snapshot.clone());
            Ok(())
        }
    }

    fn admin() -> Principal {
        Principal::new(
            Uuid::new_v4(), "root".into(), Role::Administrator, TenantScope::Global,
        )
    }

    fn version(v: &str, active: bool, snap: Option<PathBuf>, t: i64) -> InstalledVersion {
        InstalledVersion {
            id: Uuid::new_v4(),
            version: v.into(),
            package_id: None,
            installed_at_unix: t,
            is_active: active,
            snapshot_path: snap,
        }
    }

    #[test]
    fn rolls_back_to_previous_version() {
        let dir = tempdir().unwrap();
        let snap = dir.path().join("v1.0.0");
        std::fs::create_dir_all(&snap).unwrap();

        let prev = version("1.0.0", false, Some(snap.clone()), 100);
        let cur = version("1.1.0", true, None, 200);
        let repo = MockRepo {
            active: RefCell::new(Some(cur.clone())),
            rows: RefCell::new(vec![prev.clone(), cur.clone()]),
        };
        let ops = MockOps::default();
        let outcome = rollback_to_previous(&repo, &ops, &admin(), Uuid::nil()).unwrap();
        assert_eq!(outcome.from_version, "1.1.0");
        assert_eq!(outcome.to_version, "1.0.0");
        assert_eq!(*ops.restored_from.borrow(), Some(snap));
        assert_eq!(
            repo.active.borrow().as_ref().unwrap().version,
            "1.0.0"
        );
    }

    #[test]
    fn fails_with_no_previous() {
        let cur = version("1.0.0", true, None, 100);
        let repo = MockRepo {
            active: RefCell::new(Some(cur.clone())),
            rows: RefCell::new(vec![cur]),
        };
        let ops = MockOps::default();
        let err = rollback_to_previous(&repo, &ops, &admin(), Uuid::nil()).unwrap_err();
        assert!(matches!(err, RollbackError::NoPrevious));
    }

    #[test]
    fn fails_when_snapshot_missing_on_disk() {
        let prev = version("1.0.0", false, Some(PathBuf::from("/nonexistent/v1.0.0")), 100);
        let cur = version("1.1.0", true, None, 200);
        let repo = MockRepo {
            active: RefCell::new(Some(cur)),
            rows: RefCell::new(vec![prev]),
        };
        let ops = MockOps::default();
        let err = rollback_to_previous(&repo, &ops, &admin(), Uuid::nil()).unwrap_err();
        assert!(matches!(err, RollbackError::MissingSnapshot(_)));
    }
}
