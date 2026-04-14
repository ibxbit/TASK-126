//! Audit-writer contract + role mapping.
//!
//! ## Transactional guarantee
//!
//! `AuditWriter::append` is ALWAYS called from inside a business
//! transaction. The concrete SQLite implementation wraps a
//! `rusqlite::Transaction` handed in from the command handler:
//!
//! ```text
//! // command handler (future)
//! let mut conn = pool.get()?;
//! let tx = conn.transaction()?;
//!
//! let parcels = SqliteParcelRepo::bound_to(&tx);
//! let history = SqliteTransitionRepo::bound_to(&tx);
//! let audit   = SqliteAuditWriter::bound_to(&tx);
//!
//! parcel::transition(&audit, principal, &sm, &parcels, &history, input, note_enc)?;
//!
//! tx.commit()?;   // domain row AND audit row persist together.
//! ```
//!
//! If the service returns `Err`, `tx` is dropped without commit and
//! BOTH rows disappear — there are no orphan audits and no lost
//! mutations. Services therefore only need to call `append`; they
//! never manage the transaction themselves.

use crate::audit::{AuditLog, AuditRole};
use crate::auth::{Principal, Role};

pub trait AuditWriter {
    /// Append one audit row within the caller's transaction.
    fn append(&self, log: &AuditLog) -> Result<(), String>;
}

/// No-op writer. Suitable for unit tests that are not exercising
/// audit behavior. NOT wired into production code paths.
pub struct NoopAuditWriter;

impl AuditWriter for NoopAuditWriter {
    fn append(&self, _log: &AuditLog) -> Result<(), String> {
        Ok(())
    }
}

/// Map the auth-layer role onto the audit role domain.
pub fn role_to_audit(role: Role) -> AuditRole {
    match role {
        Role::Administrator => AuditRole::Administrator,
        Role::PropertyManager => AuditRole::PropertyManager,
        Role::Staff => AuditRole::Staff,
        Role::Reviewer => AuditRole::Reviewer,
        Role::Liaison => AuditRole::Liaison,
    }
}

/// Principal-aware variant. Automated actors (timeout scheduler,
/// lazy enforcer, recovery) must be attributed as `AuditRole::System`
/// so per-role auditor queries can slice "everything the system did"
/// — even though they hold `Role::Administrator` for permission
/// purposes. The convention is that their `username` is prefixed
/// with `"system:"`; `timeout::system_principal` already follows it.
pub fn audit_role_for(principal: &Principal) -> AuditRole {
    if principal.username.starts_with("system:") {
        AuditRole::System
    } else {
        role_to_audit(principal.role)
    }
}
