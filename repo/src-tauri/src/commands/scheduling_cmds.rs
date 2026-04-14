//! Scheduling IPC commands — SQLite-backed.

use std::sync::Arc;
use uuid::Uuid;

use crate::db::connection::Database;
use crate::db::repos::SqliteRuleRepo;
use crate::ipc::{guard, IpcError, SessionState};
use crate::scheduling::algorithm::{propose_schedule, Demand, Proposal};
use crate::scheduling::constraints::{validate, Assignment, ConstraintReport};
use crate::scheduling::rules::{activate_version, RuleRepository, RuleSet};

#[tauri::command]
pub fn cmd_schedule_activate_rule_set(
    session: tauri::State<'_, SessionState>,
    db: tauri::State<'_, Arc<Database>>,
    rule_set_id: Uuid,
) -> Result<(), IpcError> {
    let principal = guard::require_authenticated(session.inner())?;
    let repo = SqliteRuleRepo::new(Arc::clone(db.inner()));
    activate_version(&repo, &principal, rule_set_id)
        .map_err(|e| IpcError::Internal(e.to_string()))
}

#[tauri::command]
pub fn cmd_schedule_validate(
    session: tauri::State<'_, SessionState>,
    db: tauri::State<'_, Arc<Database>>,
    tenant_id: Uuid,
    rule_set_name: String,
    candidate: Assignment,
    existing: Vec<Assignment>,
) -> Result<ConstraintReport, IpcError> {
    guard::require_authenticated(session.inner())?;
    let repo = SqliteRuleRepo::new(Arc::clone(db.inner()));
    let rs = repo.load_active(&tenant_id, &rule_set_name)
        .map_err(|e| IpcError::Internal(e))?
        .unwrap_or_else(|| empty_rs(tenant_id, rule_set_name.clone()));
    Ok(validate(&rs, &candidate, &existing))
}

#[tauri::command]
pub fn cmd_schedule_propose(
    session: tauri::State<'_, SessionState>,
    db: tauri::State<'_, Arc<Database>>,
    tenant_id: Uuid,
    rule_set_name: String,
    demands: Vec<Demand>,
    existing: Vec<Assignment>,
    stride_seconds: i64,
) -> Result<Proposal, IpcError> {
    guard::require_authenticated(session.inner())?;
    let repo = SqliteRuleRepo::new(Arc::clone(db.inner()));
    let rs = repo.load_active(&tenant_id, &rule_set_name)
        .map_err(|e| IpcError::Internal(e))?
        .unwrap_or_else(|| empty_rs(tenant_id, rule_set_name.clone()));
    propose_schedule(&rs, &demands, &existing, stride_seconds)
        .map_err(|e| IpcError::Internal(e.to_string()))
}

fn empty_rs(tenant_id: Uuid, name: String) -> RuleSet {
    RuleSet {
        id: Uuid::nil(), tenant_id, name, version: 0,
        parent_rule_set_id: None, enabled: true, rules: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_ruleset_has_correct_defaults() {
        let tid = Uuid::new_v4();
        let rs = empty_rs(tid, "test-set".into());
        assert_eq!(rs.id, Uuid::nil());
        assert_eq!(rs.tenant_id, tid);
        assert_eq!(rs.name, "test-set");
        assert_eq!(rs.version, 0);
        assert!(rs.enabled);
        assert!(rs.rules.is_empty());
    }

    #[test]
    fn validate_with_empty_ruleset_produces_no_hard_violations() {
        let tid = Uuid::new_v4();
        let rs = empty_rs(tid, "empty".into());
        let candidate = Assignment {
            resource_id: Uuid::new_v4(),
            subject_id: None,
            window: crate::scheduling::rules::TimeWindow {
                start_unix: 1000,
                end_unix: 2000,
            },
        };
        let report = validate(&rs, &candidate, &[]);
        assert!(report.hard_violations.is_empty(), "no rules means no hard violations");
    }

    #[test]
    fn propose_with_empty_demands_returns_empty_proposal() {
        let tid = Uuid::new_v4();
        let rs = empty_rs(tid, "empty".into());
        let proposal = propose_schedule(&rs, &[], &[], 3600).unwrap();
        assert!(proposal.assigned.is_empty());
        assert!(proposal.unfulfilled.is_empty());
    }
}
