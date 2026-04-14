//! Role-based, tenant-scoped access control.
//!
//! All permission enforcement lives in the Rust backend. The UI must
//! never be treated as authoritative for access decisions; every
//! `#[tauri::command]` that mutates or reads scoped data MUST call
//! `guard::require(...)` before proceeding.

pub mod assignment;
pub mod context;
pub mod guard;
pub mod permissions;
pub mod roles;

pub use assignment::{assign_role, revoke_role, validate_assignment, AssignmentError};
pub use context::{Principal, TenantScope};
pub use guard::{require, require_any, AuthError};
pub use permissions::Permission;
pub use roles::Role;
