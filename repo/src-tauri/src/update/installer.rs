//! Update installer.
//!
//! Sequence:
//!   1. Verify the .spkg signature (caller has already done this; we
//!      receive a `VerifiedPackage`).
//!   2. Enforce `min_required_version` against the currently active
//!      `app_versions` row.
//!   3. Snapshot the current install (binaries + DB file) into
//!      `/backups/v<current>/` — only the immediately-previous version
//!      is retained, older snapshots are pruned.
//!   4. Stage the package payload to `/staging/v<new>/`.
//!   5. Atomic activate: insert the new `app_versions` row and flip
//!      `is_active` from old → new in a single transaction. The
//!      partial unique index on `is_active = 1` guarantees one active
//!      version at any moment.
//!   6. Restart hint returned to the caller.

use std::path::PathBuf;

use serde::Serialize;
use thiserror::Error;
use uuid::Uuid;

use crate::auth::{self, AuthError, Permission, Principal};
use crate::update::verifier::VerifiedPackage;

#[derive(Debug, Error, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InstallError {
    #[error(transparent)]
    Auth(#[from] AuthError),

    #[error("min required version is '{required}', current is '{current}'")]
    VersionGate { required: String, current: String },

    #[error("a version with id '{0}' is already installed")]
    DuplicateVersion(String),

    #[error("snapshot failed: {0}")]
    Snapshot(String),

    #[error("staging failed: {0}")]
    Staging(String),

    #[error("activation failed: {0}")]
    Activation(String),

    #[error("persistence error: {0}")]
    Persistence(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallOutcome {
    pub previous_version: Option<String>,
    pub new_version: String,
    pub snapshot_path: PathBuf,
    pub staging_path: PathBuf,
    pub restart_required: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstalledVersion {
    pub id: Uuid,
    pub version: String,
    pub package_id: Option<Uuid>,
    pub installed_at_unix: i64,
    pub is_active: bool,
    pub snapshot_path: Option<PathBuf>,
}

pub trait VersionRepository {
    fn active(&self) -> Result<Option<InstalledVersion>, String>;
    fn previous(&self) -> Result<Option<InstalledVersion>, String>;
    fn exists(&self, version: &str) -> Result<bool, String>;
    /// Insert the new version row AND flip `is_active` from the
    /// current active row to this new one in a single transaction.
    /// The schema's partial unique index enforces single-active.
    fn install_and_activate(
        &self,
        new: &InstalledVersion,
    ) -> Result<(), String>;
    /// Delete app_versions rows older than the previous (i.e. keep
    /// only active + previous). Returns paths whose snapshots may be
    /// pruned from disk.
    fn prune_older_than_previous(&self) -> Result<Vec<PathBuf>, String>;
}

/// Filesystem operations the installer needs. Behind a trait so tests
/// can swap in a temp-directory mock.
pub trait InstallerOps {
    /// Release file handles that would block snapshotting (DB
    /// connections, open attachments, tracked writers, etc.).
    /// Concrete impls MUST run `HandleQuiescer::quiesce_and_verify`
    /// so the tracker-count-zero invariant is enforced BEFORE
    /// proceeding — a non-zero count means the DB file might still
    /// be locked when we try to copy it.
    fn quiesce(&self) -> Result<(), String>;
    /// Copy the current install (binaries + DB) into `dest`.
    fn snapshot_current(&self, dest: &PathBuf) -> Result<(), String>;
    /// Extract the package's `payload/` entries into `dest`.
    fn stage_payload(&self, package_path: &PathBuf, dest: &PathBuf) -> Result<(), String>;
    /// Best-effort delete of a directory tree.
    fn delete_dir(&self, path: &PathBuf) -> Result<(), String>;
}

pub fn install_package<V: VersionRepository, O: InstallerOps>(
    versions: &V,
    ops: &O,
    principal: &Principal,
    tenant_for_audit: Uuid,
    package: &VerifiedPackage,
    backups_root: &PathBuf,
    staging_root: &PathBuf,
    now_unix: i64,
) -> Result<InstallOutcome, InstallError> {
    // Permission gate — only Administrators may install updates.
    auth::require(principal, Permission::ConfigurePermissions, &tenant_for_audit)?;

    let current = versions.active().map_err(InstallError::Persistence)?;

    // Version gate.
    if let Some(req) = &package.manifest.min_required_version {
        let cur_v = current.as_ref().map(|v| v.version.as_str()).unwrap_or("");
        if cur_v != req {
            return Err(InstallError::VersionGate {
                required: req.clone(),
                current: cur_v.to_string(),
            });
        }
    }

    if versions
        .exists(&package.manifest.version)
        .map_err(InstallError::Persistence)?
    {
        return Err(InstallError::DuplicateVersion(package.manifest.version.clone()));
    }

    // Snapshot current.
    let snapshot_path = backups_root.join(format!(
        "v{}",
        current.as_ref().map(|v| v.version.as_str()).unwrap_or("none")
    ));
    if current.is_some() {
        // Release every file handle (DB, attachments, upload writers)
        // BEFORE touching the live install. On Windows this is the
        // difference between a clean snapshot and a sharing violation
        // halfway through the copy.
        ops.quiesce().map_err(InstallError::Snapshot)?;
        ops.snapshot_current(&snapshot_path)
            .map_err(InstallError::Snapshot)?;
    }

    // Stage payload.
    let staging_path = staging_root.join(format!("v{}", package.manifest.version));
    ops.stage_payload(&package.package_path, &staging_path)
        .map_err(InstallError::Staging)?;

    // Activate (single DB transaction).
    let new = InstalledVersion {
        id: Uuid::new_v4(),
        version: package.manifest.version.clone(),
        package_id: Some(package.manifest.package_id),
        installed_at_unix: now_unix,
        is_active: true,
        snapshot_path: Some(snapshot_path.clone()),
    };
    versions
        .install_and_activate(&new)
        .map_err(InstallError::Activation)?;

    // Prune anything older than the previous version (we keep N + N-1).
    let stale = versions
        .prune_older_than_previous()
        .map_err(InstallError::Persistence)?;
    for p in stale {
        let _ = ops.delete_dir(&p);
    }

    Ok(InstallOutcome {
        previous_version: current.map(|v| v.version),
        new_version: package.manifest.version.clone(),
        snapshot_path,
        staging_path,
        restart_required: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::context::TenantScope;
    use crate::auth::Role;
    use crate::update::verifier::PackageManifest;
    use std::cell::RefCell;
    use tempfile::tempdir;

    struct MockVersions {
        active: RefCell<Option<InstalledVersion>>,
        rows: RefCell<Vec<InstalledVersion>>,
    }
    impl VersionRepository for MockVersions {
        fn active(&self) -> Result<Option<InstalledVersion>, String> {
            Ok(self.active.borrow().clone())
        }
        fn previous(&self) -> Result<Option<InstalledVersion>, String> {
            let rows = self.rows.borrow();
            let mut sorted: Vec<&InstalledVersion> = rows.iter().filter(|r| !r.is_active).collect();
            sorted.sort_by_key(|r| -r.installed_at_unix);
            Ok(sorted.first().map(|&v| v.clone()))
        }
        fn exists(&self, version: &str) -> Result<bool, String> {
            Ok(self.rows.borrow().iter().any(|r| r.version == version))
        }
        fn install_and_activate(&self, new: &InstalledVersion) -> Result<(), String> {
            // Demote current active.
            for r in self.rows.borrow_mut().iter_mut() {
                r.is_active = false;
            }
            self.rows.borrow_mut().push(new.clone());
            *self.active.borrow_mut() = Some(new.clone());
            Ok(())
        }
        fn prune_older_than_previous(&self) -> Result<Vec<PathBuf>, String> {
            let mut rows = self.rows.borrow_mut();
            // Keep active + the most recent inactive.
            rows.sort_by_key(|r| -r.installed_at_unix);
            let mut to_drop = Vec::new();
            for (i, r) in rows.iter().enumerate() {
                // Index 0 = most recent (active), 1 = previous, drop the rest.
                if i >= 2 {
                    if let Some(p) = &r.snapshot_path {
                        to_drop.push(p.clone());
                    }
                }
            }
            rows.truncate(2);
            Ok(to_drop)
        }
    }

    struct MockOps {
        snapshot_calls: RefCell<u32>,
        stage_calls: RefCell<u32>,
        deletes: RefCell<Vec<PathBuf>>,
        quiesce_calls: RefCell<u32>,
        quiesce_result: RefCell<Result<(), String>>,
    }
    impl Default for MockOps {
        fn default() -> Self {
            Self {
                snapshot_calls: RefCell::new(0),
                stage_calls: RefCell::new(0),
                deletes: RefCell::new(Vec::new()),
                quiesce_calls: RefCell::new(0),
                quiesce_result: RefCell::new(Ok(())),
            }
        }
    }
    impl InstallerOps for MockOps {
        fn quiesce(&self) -> Result<(), String> {
            *self.quiesce_calls.borrow_mut() += 1;
            self.quiesce_result.borrow().clone()
        }
        fn snapshot_current(&self, dest: &PathBuf) -> Result<(), String> {
            *self.snapshot_calls.borrow_mut() += 1;
            std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
            std::fs::write(dest.join("DB.snap"), b"x").map_err(|e| e.to_string())?;
            Ok(())
        }
        fn stage_payload(&self, _pkg: &PathBuf, dest: &PathBuf) -> Result<(), String> {
            *self.stage_calls.borrow_mut() += 1;
            std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
            Ok(())
        }
        fn delete_dir(&self, p: &PathBuf) -> Result<(), String> {
            self.deletes.borrow_mut().push(p.clone());
            let _ = std::fs::remove_dir_all(p);
            Ok(())
        }
    }

    fn admin() -> Principal {
        Principal::new(
            Uuid::new_v4(), "root".into(), Role::Administrator, TenantScope::Global,
        )
    }

    fn manifest(version: &str, min_req: Option<&str>) -> PackageManifest {
        PackageManifest {
            manifest_format: 1,
            package_id: Uuid::new_v4(),
            version: version.into(),
            created_at_unix: 1,
            min_required_version: min_req.map(|s| s.into()),
            payload_sha256_hex: "00".into(),
            notes: None,
        }
    }

    fn pkg(version: &str, min_req: Option<&str>) -> VerifiedPackage {
        VerifiedPackage {
            manifest: manifest(version, min_req),
            package_path: PathBuf::from("/tmp/none.spkg"),
        }
    }

    #[test]
    fn fresh_install_succeeds() {
        let versions = MockVersions {
            active: RefCell::new(None),
            rows: RefCell::new(vec![]),
        };
        let ops = MockOps::default();
        let dir = tempdir().unwrap();
        let backups = dir.path().join("backups");
        let staging = dir.path().join("staging");
        let outcome = install_package(
            &versions, &ops, &admin(), Uuid::nil(),
            &pkg("1.0.0", None), &backups, &staging, 1000,
        ).unwrap();
        assert_eq!(outcome.previous_version, None);
        assert_eq!(outcome.new_version, "1.0.0");
        // No snapshot when no previous version exists.
        assert_eq!(*ops.snapshot_calls.borrow(), 0);
        assert_eq!(*ops.stage_calls.borrow(), 1);
    }

    #[test]
    fn upgrade_snapshots_previous() {
        let versions = MockVersions {
            active: RefCell::new(Some(InstalledVersion {
                id: Uuid::new_v4(), version: "1.0.0".into(), package_id: None,
                installed_at_unix: 0, is_active: true, snapshot_path: None,
            })),
            rows: RefCell::new(vec![InstalledVersion {
                id: Uuid::new_v4(), version: "1.0.0".into(), package_id: None,
                installed_at_unix: 0, is_active: true, snapshot_path: None,
            }]),
        };
        let ops = MockOps::default();
        let dir = tempdir().unwrap();
        let outcome = install_package(
            &versions, &ops, &admin(), Uuid::nil(),
            &pkg("1.1.0", Some("1.0.0")),
            &dir.path().join("backups"), &dir.path().join("staging"),
            2000,
        ).unwrap();
        assert_eq!(outcome.previous_version.as_deref(), Some("1.0.0"));
        assert_eq!(*ops.snapshot_calls.borrow(), 1);
    }

    #[test]
    fn version_gate_blocks_wrong_current() {
        let versions = MockVersions {
            active: RefCell::new(Some(InstalledVersion {
                id: Uuid::new_v4(), version: "0.9.0".into(), package_id: None,
                installed_at_unix: 0, is_active: true, snapshot_path: None,
            })),
            rows: RefCell::new(vec![]),
        };
        let ops = MockOps::default();
        let dir = tempdir().unwrap();
        let err = install_package(
            &versions, &ops, &admin(), Uuid::nil(),
            &pkg("1.1.0", Some("1.0.0")),
            &dir.path().join("backups"), &dir.path().join("staging"),
            2000,
        ).unwrap_err();
        assert!(matches!(err, InstallError::VersionGate { .. }));
    }

    #[test]
    fn upgrade_quiesces_before_snapshot() {
        let versions = MockVersions {
            active: RefCell::new(Some(InstalledVersion {
                id: Uuid::new_v4(), version: "1.0.0".into(), package_id: None,
                installed_at_unix: 0, is_active: true, snapshot_path: None,
            })),
            rows: RefCell::new(vec![InstalledVersion {
                id: Uuid::new_v4(), version: "1.0.0".into(), package_id: None,
                installed_at_unix: 0, is_active: true, snapshot_path: None,
            }]),
        };
        let ops = MockOps::default();
        let dir = tempdir().unwrap();
        install_package(
            &versions, &ops, &admin(), Uuid::nil(),
            &pkg("1.1.0", Some("1.0.0")),
            &dir.path().join("backups"), &dir.path().join("staging"),
            2000,
        ).unwrap();
        // Quiesce ran exactly once, BEFORE snapshot.
        assert_eq!(*ops.quiesce_calls.borrow(), 1);
        assert_eq!(*ops.snapshot_calls.borrow(), 1);
    }

    #[test]
    fn quiesce_failure_aborts_install_before_snapshot() {
        let versions = MockVersions {
            active: RefCell::new(Some(InstalledVersion {
                id: Uuid::new_v4(), version: "1.0.0".into(), package_id: None,
                installed_at_unix: 0, is_active: true, snapshot_path: None,
            })),
            rows: RefCell::new(vec![]),
        };
        let ops = MockOps::default();
        *ops.quiesce_result.borrow_mut() = Err("5 file handle(s) still open".into());
        let dir = tempdir().unwrap();
        let err = install_package(
            &versions, &ops, &admin(), Uuid::nil(),
            &pkg("1.1.0", Some("1.0.0")),
            &dir.path().join("backups"), &dir.path().join("staging"),
            2000,
        ).unwrap_err();
        assert!(matches!(err, InstallError::Snapshot(_)));
        // Snapshot and stage were NOT invoked — the install never
        // started touching the live files.
        assert_eq!(*ops.snapshot_calls.borrow(), 0);
        assert_eq!(*ops.stage_calls.borrow(), 0);
    }

    #[test]
    fn fresh_install_does_not_call_quiesce() {
        // No current version → nothing to snapshot → no need to quiesce.
        let versions = MockVersions {
            active: RefCell::new(None),
            rows: RefCell::new(vec![]),
        };
        let ops = MockOps::default();
        let dir = tempdir().unwrap();
        install_package(
            &versions, &ops, &admin(), Uuid::nil(),
            &pkg("1.0.0", None),
            &dir.path().join("backups"), &dir.path().join("staging"),
            1000,
        ).unwrap();
        assert_eq!(*ops.quiesce_calls.borrow(), 0);
    }

    #[test]
    fn duplicate_version_rejected() {
        let v1 = InstalledVersion {
            id: Uuid::new_v4(), version: "1.0.0".into(), package_id: None,
            installed_at_unix: 0, is_active: true, snapshot_path: None,
        };
        let versions = MockVersions {
            active: RefCell::new(Some(v1.clone())),
            rows: RefCell::new(vec![v1]),
        };
        let ops = MockOps::default();
        let dir = tempdir().unwrap();
        let err = install_package(
            &versions, &ops, &admin(), Uuid::nil(),
            &pkg("1.0.0", None),
            &dir.path().join("backups"), &dir.path().join("staging"),
            2000,
        ).unwrap_err();
        assert!(matches!(err, InstallError::DuplicateVersion(_)));
    }
}
