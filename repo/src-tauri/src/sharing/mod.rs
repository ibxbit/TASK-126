//! Data protection & local sharing: download watermarks,
//! password-protected share packages, and expiry enforcement.

pub mod expiry;
pub mod package;
pub mod watermark;

pub use expiry::{
    revoke_package, sweep_expired, verify_access, ExpiryError, ExpirySweeper, PackageRecord,
    PackageRepository, DEFAULT_LIFETIME_SECONDS,
};
pub use package::{
    build_share_package, PackageBuildInput, PackageBuildOutcome, PackageError, PackageItem,
};
pub use watermark::{wrap_with_watermark, WatermarkError, WatermarkSpec};
