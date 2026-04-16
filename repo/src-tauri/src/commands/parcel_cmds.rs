//! Parcel IPC commands — SQLite-backed.

use std::sync::Arc;
use uuid::Uuid;

use crate::db::connection::Database;
use crate::db::repos::{SqliteAuditWriter, SqliteParcelRepo, SqliteTransitionRepo};
use crate::ipc::{guard, IpcError, SessionState};
use crate::auth::Permission;
use crate::parcel::machine::{default_guards, default_rules, StateMachine};
use crate::parcel::state::ParcelState;
use crate::parcel::transition::{TransitionInput, TransitionRecord};

pub fn now_unix_export() -> i64 { now_unix() }

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[tauri::command]
pub fn cmd_parcel_available_transitions(
    session: tauri::State<'_, SessionState>,
    current: Option<ParcelState>,
) -> Result<Vec<ParcelState>, IpcError> {
    guard::require_authenticated(session.inner())?;
    let sm = StateMachine::new(default_rules(), default_guards());
    Ok(sm.available_from(current))
}

#[tauri::command]
pub fn cmd_transition_parcel(
    session: tauri::State<'_, SessionState>,
    db: tauri::State<'_, Arc<Database>>,
    input: TransitionInput,
) -> Result<TransitionRecord, IpcError> {
    let principal = guard::require(session.inner(), Permission::ParcelOperate, &input.tenant_id)?;
    let parcel_repo = SqliteParcelRepo::new(Arc::clone(db.inner()));
    let trans_repo = SqliteTransitionRepo::new(Arc::clone(db.inner()));
    let audit = SqliteAuditWriter::new(Arc::clone(db.inner()));
    let sm = StateMachine::new(default_rules(), default_guards());

    crate::parcel::transition::transition(
        &audit,
        &principal,
        &sm,
        &parcel_repo,
        &trans_repo,
        input,
        None,
    )
    .map_err(|e| IpcError::Internal(format!("{e:?}")))
}

#[tauri::command]
pub fn cmd_parcel_history(
    session: tauri::State<'_, SessionState>,
    db: tauri::State<'_, Arc<Database>>,
    parcel_id: Uuid,
) -> Result<Vec<TransitionRecord>, IpcError> {
    guard::require_authenticated(session.inner())?;
    let repo = SqliteTransitionRepo::new(Arc::clone(db.inner()));
    use crate::parcel::transition::TransitionRepository;
    repo.history(&parcel_id).map_err(|e| IpcError::Internal(e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parcel::machine::default_guards;

    #[test]
    fn now_unix_returns_positive() {
        assert!(now_unix() > 0);
    }

    #[test]
    fn default_state_machine_has_available_transitions_from_none() {
        let sm = StateMachine::new(default_rules(), default_guards());
        let avail = sm.available_from(None);
        assert!(!avail.is_empty(), "should have transitions from initial state");
        assert!(avail.contains(&ParcelState::CheckedIn));
    }

    #[test]
    fn default_state_machine_checked_in_has_transitions() {
        let sm = StateMachine::new(default_rules(), default_guards());
        let avail = sm.available_from(Some(ParcelState::CheckedIn));
        assert!(!avail.is_empty());
    }

    #[test]
    fn default_state_machine_delivered_has_no_check_in_transition() {
        let sm = StateMachine::new(default_rules(), default_guards());
        let avail = sm.available_from(Some(ParcelState::Delivered));
        // Delivered should not be able to go back to CheckedIn.
        assert!(!avail.contains(&ParcelState::CheckedIn));
    }

    // ── End-to-end integration tests (IPC contract ring) ──────────────
    //
    // These exercise the same call chain `cmd_transition_parcel` and
    // `cmd_parcel_history` use, against a real SQLite DB. Verifies the
    // command body — repo wiring, audit row, history shape — would
    // succeed if invoked from Tauri.

    use crate::auth::context::{Principal, TenantScope};
    use crate::auth::Role;
    use crate::db::repos::{SqliteAuditWriter, SqliteParcelRepo, SqliteTransitionRepo};
    use crate::ipc::SessionState;
    use crate::parcel::transition::{TransitionInput, TransitionRepository};

    fn db_with_migrations() -> Arc<crate::db::connection::Database> {
        let db =
            crate::db::connection::Database::open_in_memory().expect("open");
        db.run_migrations().expect("migrate");
        Arc::new(db)
    }

    fn seed_parcel(
        db: &Arc<crate::db::connection::Database>,
        tid: Uuid,
    ) -> (Uuid, Uuid) {
        let now = 1_700_000_000i64;
        let uid = Uuid::new_v4();
        let pid = Uuid::new_v4();
        let rid = Uuid::new_v4();
        let c = db.conn();
        c.execute(
            "INSERT INTO tenants (id, name, code, active, created_at, updated_at)
             VALUES (?1, 'T', ?2, 1, ?3, ?3)",
            rusqlite::params![tid.to_string(), tid.to_string(), now],
        )
        .unwrap();
        c.execute(
            "INSERT INTO users (id, username, display_name, password_hash, active, created_at, updated_at)
             VALUES (?1, ?2, ?2, '$argon2id$v=19$m=16,t=2,p=1$dGVzdHNhbHQ$abc', 1, ?3, ?3)",
            rusqlite::params![uid.to_string(), "operator", now],
        )
        .unwrap();
        c.execute(
            "INSERT INTO residents (id, tenant_id, full_name, created_at, updated_at, created_by)
             VALUES (?1, ?2, 'R', ?3, ?3, ?4)",
            rusqlite::params![rid.to_string(), tid.to_string(), now, uid.to_string()],
        )
        .unwrap();
        c.execute(
            "INSERT INTO parcels (id, tenant_id, resident_id, status, received_at, created_at, updated_at, created_by)
             VALUES (?1, ?2, ?3, 'checked_in', ?4, ?4, ?4, ?5)",
            rusqlite::params![pid.to_string(), tid.to_string(), rid.to_string(), now, uid.to_string()],
        )
        .unwrap();
        (pid, uid)
    }

    fn pm(uid: Uuid, tid: Uuid) -> Principal {
        Principal::new(
            uid,
            "operator".into(),
            Role::PropertyManager,
            TenantScope::single(tid),
        )
    }

    /// Invoke the same call chain as `cmd_transition_parcel` but
    /// without `tauri::State`.
    fn invoke_transition(
        db: &Arc<crate::db::connection::Database>,
        principal: &Principal,
        input: TransitionInput,
    ) -> Result<crate::parcel::transition::TransitionRecord, IpcError> {
        let parcel_repo = SqliteParcelRepo::new(Arc::clone(db));
        let trans_repo = SqliteTransitionRepo::new(Arc::clone(db));
        let audit = SqliteAuditWriter::new(Arc::clone(db));
        let sm = StateMachine::new(default_rules(), default_guards());
        crate::parcel::transition::transition(
            &audit, principal, &sm, &parcel_repo, &trans_repo, input, None,
        )
        .map_err(|e| IpcError::Internal(format!("{e:?}")))
    }

    #[test]
    fn transition_writes_history_row_and_audit_row() {
        let db = db_with_migrations();
        let tid = Uuid::new_v4();
        let (pid, uid) = seed_parcel(&db, tid);
        let p = pm(uid, tid);

        let input = TransitionInput {
            parcel_id: pid,
            tenant_id: tid,
            to_state: ParcelState::CheckedOut,
            location: "Front Desk".into(),
            occurred_at_unix: Some(1_700_000_001),
            notes: None,
        };
        invoke_transition(&db, &p, input).expect("transition ok");

        let trans_repo = SqliteTransitionRepo::new(Arc::clone(&db));
        let history = trans_repo.history(&pid).expect("history");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].to_state, ParcelState::CheckedOut);
        assert_eq!(history[0].location, "Front Desk");
        // Hash chain anchors at None for the first transition.
        assert!(history[0].prev_chain_hash.is_none());
        assert!(!history[0].chain_hash.is_empty());

        let audit_count: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM audit_logs", [], |r| r.get(0))
            .unwrap();
        assert!(audit_count >= 1, "audit row must be appended");
    }

    #[test]
    fn unauthenticated_session_blocks_command_through_guard() {
        // Direct guard check — exercises the same gate cmd_* use.
        let session = SessionState::new();
        let res = guard::require_authenticated(&session);
        assert!(matches!(res, Err(IpcError::Unauthenticated)));
    }

    #[test]
    fn liaison_is_blocked_from_parcel_operate() {
        // Permission check at the IPC guard layer.
        let tid = Uuid::new_v4();
        let session = SessionState::new();
        let p = Principal::new(
            Uuid::new_v4(),
            "liaison".into(),
            Role::Liaison,
            TenantScope::single(tid),
        );
        session.set(p);
        let res = guard::require(&session, crate::auth::Permission::ParcelOperate, &tid);
        match res {
            Err(IpcError::PermissionDenied { role, permission }) => {
                assert_eq!(role, "liaison");
                assert_eq!(permission, "parcel_operate");
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn cross_tenant_request_is_blocked_at_guard() {
        let allowed = Uuid::new_v4();
        let foreign = Uuid::new_v4();
        let session = SessionState::new();
        session.set(Principal::new(
            Uuid::new_v4(),
            "u".into(),
            Role::PropertyManager,
            TenantScope::single(allowed),
        ));
        let res = guard::require(&session, crate::auth::Permission::ParcelOperate, &foreign);
        assert!(matches!(res, Err(IpcError::TenantScopeViolation { .. })));
    }
}
