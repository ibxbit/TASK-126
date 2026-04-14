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
}
