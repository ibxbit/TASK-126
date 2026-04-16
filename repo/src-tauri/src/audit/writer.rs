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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::{AuditLog, NewAuditLog};
    use crate::auth::context::TenantScope;
    use uuid::Uuid;

    #[test]
    fn role_to_audit_is_exhaustive_for_every_role() {
        let cases = [
            (Role::Administrator,    AuditRole::Administrator),
            (Role::PropertyManager,  AuditRole::PropertyManager),
            (Role::Staff,            AuditRole::Staff),
            (Role::Reviewer,         AuditRole::Reviewer),
            (Role::Liaison,          AuditRole::Liaison),
        ];
        for (auth, expect) in cases {
            assert_eq!(role_to_audit(auth), expect, "{:?}", auth);
        }
    }

    fn principal_named(name: &str, role: Role) -> Principal {
        Principal::new(Uuid::new_v4(), name.into(), role, TenantScope::Global)
    }

    #[test]
    fn audit_role_for_human_user_returns_their_role() {
        let p = principal_named("alice", Role::PropertyManager);
        assert_eq!(audit_role_for(&p), AuditRole::PropertyManager);
    }

    #[test]
    fn audit_role_for_system_username_returns_system_even_when_role_is_admin() {
        // The timeout scheduler runs with Administrator-level permission
        // but the audit row must be attributed to System.
        let p = principal_named("system:claim_timeout", Role::Administrator);
        assert_eq!(audit_role_for(&p), AuditRole::System);
    }

    #[test]
    fn audit_role_for_username_just_containing_system_is_not_system() {
        // The marker is *prefix*, not substring — guard against false positives.
        let p = principal_named("subsystem.bot", Role::Staff);
        assert_eq!(audit_role_for(&p), AuditRole::Staff);
    }

    #[test]
    fn noop_audit_writer_returns_ok_without_side_effects() {
        let writer = NoopAuditWriter;
        let log = AuditLog::new(
            NewAuditLog {
                user_id: Uuid::new_v4(),
                role: AuditRole::Staff,
                tenant_id: None,
                action_type: "x".into(),
                entity_type: "y".into(),
                entity_id: None,
                before_state: None,
                after_state: None,
                metadata: None,
            },
            0,
        );
        writer.append(&log).expect("noop never errors");
    }

    #[test]
    fn audit_role_serializes_as_snake_case_string() {
        let json = serde_json::to_string(&AuditRole::PropertyManager).unwrap();
        assert_eq!(json, r#""property_manager""#);
        let json = serde_json::to_string(&AuditRole::System).unwrap();
        assert_eq!(json, r#""system""#);
    }
}
