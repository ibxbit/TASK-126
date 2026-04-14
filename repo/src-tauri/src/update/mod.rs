//! Offline update + rollback subsystem.

pub mod installer;
pub mod rollback;
pub mod verifier;

pub use installer::{
    install_package, InstallError, InstallOutcome, InstallerOps, VersionRepository,
};
pub use rollback::{rollback_to_previous, RollbackError, RollbackOutcome};
pub use verifier::{
    canonical_manifest_bytes, verify_package, PackageManifest, VerifiedPackage, VerifyError,
};
