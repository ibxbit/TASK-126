//! Settlement state machine.
//!
//!   draft ─Prepare→ pending_approval ─Approve→ approved ─MarkPaid→ paid
//!     │                  │                       │
//!     │                  └────Withdraw───────────┘
//!     │                                          │
//!     └────────────────Void──────────────────────┘
//!                                                ▼
//!                                            reopened ─Prepare→ pending_approval
//!
//! Every transition is enforced through `apply_event` which routes
//! permission checks through `auth::guard::require` and persistence
//! through `SettlementRepository`.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::auth::{self, AuthError, Permission, Principal};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettlementStatus {
    Draft,
    PendingApproval,
    Approved,
    Paid,
    Reopened,
    Void,
}

impl SettlementStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            SettlementStatus::Draft => "draft",
            SettlementStatus::PendingApproval => "pending_approval",
            SettlementStatus::Approved => "approved",
            SettlementStatus::Paid => "paid",
            SettlementStatus::Reopened => "reopened",
            SettlementStatus::Void => "void",
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, SettlementStatus::Paid | SettlementStatus::Void)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum SettlementEvent {
    /// Preparer signs off — line items are now locked.
    Prepare,
    /// Approver signs off — must be a different user.
    Approve,
    /// Either preparer or approver pulls back to Draft.
    Withdraw,
    /// Treasurer marks the check as cut + ledger posted.
    MarkPaid,
    /// Manager voids the settlement (terminal).
    Void,
    /// Manager re-opens a previously approved settlement.
    Reopen,
}

#[derive(Debug, Clone)]
pub struct SettlementView {
    pub settlement_id: Uuid,
    pub tenant_id: Uuid,
    pub status: SettlementStatus,
    pub prepared_by: Option<Uuid>,
    pub approved_by: Option<Uuid>,
}

#[derive(Debug, Error, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkflowError {
    #[error(transparent)]
    Auth(#[from] AuthError),

    #[error("event '{event}' not permitted from status '{status}'")]
    IllegalTransition { event: &'static str, status: String },

    #[error("settlement is in terminal status '{0}'")]
    Terminal(String),

    #[error("settlement must be prepared before approval")]
    NotPrepared,

    #[error("approver must differ from preparer")]
    SamePartyApproval,

    #[error("only the preparer or an approver may withdraw")]
    NotAuthorizedToWithdraw,

    #[error("persistence error: {0}")]
    Persistence(String),
}

pub trait SettlementRepository {
    fn load(&self, settlement_id: &Uuid) -> Result<Option<SettlementView>, String>;
    fn set_status(
        &self,
        settlement_id: &Uuid,
        new_status: SettlementStatus,
        actor: &Principal,
        now_unix: i64,
    ) -> Result<(), String>;
}

fn event_name(e: &SettlementEvent) -> &'static str {
    match e {
        SettlementEvent::Prepare => "prepare",
        SettlementEvent::Approve => "approve",
        SettlementEvent::Withdraw => "withdraw",
        SettlementEvent::MarkPaid => "mark_paid",
        SettlementEvent::Void => "void",
        SettlementEvent::Reopen => "reopen",
    }
}

fn required_permission(e: &SettlementEvent) -> Permission {
    use SettlementEvent::*;
    match e {
        Prepare => Permission::ApproveSettlement, // Property Manager prepares
        Approve => Permission::ApproveSettlement,
        Withdraw => Permission::ApproveSettlement,
        MarkPaid => Permission::ApproveSettlement,
        Void => Permission::ApproveSettlement,
        Reopen => Permission::ReopenClaim,
    }
}

/// Pure transition table.
fn next_status(
    view: &SettlementView,
    event: &SettlementEvent,
    actor: &Principal,
) -> Result<SettlementStatus, WorkflowError> {
    use SettlementEvent::*;
    use SettlementStatus::*;

    if view.status.is_terminal() && !matches!(event, Reopen) {
        return Err(WorkflowError::Terminal(view.status.as_str().to_string()));
    }

    match (view.status, event) {
        (Draft, Prepare) | (Reopened, Prepare) => Ok(PendingApproval),

        (PendingApproval, Approve) => {
            // Two-step: approver MUST differ from preparer.
            if let Some(prep) = view.prepared_by {
                if prep == actor.user_id {
                    return Err(WorkflowError::SamePartyApproval);
                }
            } else {
                return Err(WorkflowError::NotPrepared);
            }
            Ok(Approved)
        }

        (PendingApproval, Withdraw) | (Approved, Withdraw) => {
            // Only preparer or approver may pull it back.
            let ok = view
                .prepared_by
                .map(|u| u == actor.user_id)
                .unwrap_or(false)
                || view
                    .approved_by
                    .map(|u| u == actor.user_id)
                    .unwrap_or(false);
            if !ok {
                return Err(WorkflowError::NotAuthorizedToWithdraw);
            }
            Ok(Draft)
        }

        (Approved, MarkPaid) => Ok(Paid),
        (_, SettlementEvent::Void) if !view.status.is_terminal() => Ok(SettlementStatus::Void),
        (_, Reopen) if view.status.is_terminal() => Ok(Reopened),

        (status, ev) => Err(WorkflowError::IllegalTransition {
            event: event_name(ev),
            status: status.as_str().to_string(),
        }),
    }
}

/// Orchestrate a transition: load → validate → auth → persist.
pub fn apply_event<R: SettlementRepository>(
    repo: &R,
    principal: &Principal,
    settlement_id: &Uuid,
    event: SettlementEvent,
    now_unix: i64,
) -> Result<SettlementStatus, WorkflowError> {
    let view = repo
        .load(settlement_id)
        .map_err(WorkflowError::Persistence)?
        .ok_or(WorkflowError::Persistence("settlement not found".into()))?;

    let new_status = next_status(&view, &event, principal)?;

    auth::require(principal, required_permission(&event), &view.tenant_id)?;

    repo.set_status(settlement_id, new_status, principal, now_unix)
        .map_err(WorkflowError::Persistence)?;

    Ok(new_status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::context::TenantScope;
    use crate::auth::Role;

    fn pm(uid: Uuid, tenant: Uuid) -> Principal {
        Principal::new(uid, "pm".into(), Role::PropertyManager, TenantScope::single(tenant))
    }

    fn view(status: SettlementStatus, prepared: Option<Uuid>, approved: Option<Uuid>, tenant: Uuid) -> SettlementView {
        SettlementView {
            settlement_id: Uuid::new_v4(),
            tenant_id: tenant,
            status,
            prepared_by: prepared,
            approved_by: approved,
        }
    }

    #[test]
    fn draft_to_pending_approval() {
        let t = Uuid::new_v4();
        let actor = pm(Uuid::new_v4(), t);
        let v = view(SettlementStatus::Draft, None, None, t);
        assert_eq!(next_status(&v, &SettlementEvent::Prepare, &actor).unwrap(),
                   SettlementStatus::PendingApproval);
    }

    #[test]
    fn approver_must_differ_from_preparer() {
        let t = Uuid::new_v4();
        let prep = Uuid::new_v4();
        let actor = pm(prep, t); // same user
        let v = view(SettlementStatus::PendingApproval, Some(prep), None, t);
        let err = next_status(&v, &SettlementEvent::Approve, &actor).unwrap_err();
        assert!(matches!(err, WorkflowError::SamePartyApproval));
    }

    #[test]
    fn approval_succeeds_with_distinct_approver() {
        let t = Uuid::new_v4();
        let prep = Uuid::new_v4();
        let appr = pm(Uuid::new_v4(), t);
        let v = view(SettlementStatus::PendingApproval, Some(prep), None, t);
        assert_eq!(next_status(&v, &SettlementEvent::Approve, &appr).unwrap(),
                   SettlementStatus::Approved);
    }

    #[test]
    fn paid_is_terminal_and_blocks_further_events() {
        let t = Uuid::new_v4();
        let actor = pm(Uuid::new_v4(), t);
        let v = view(SettlementStatus::Paid, Some(Uuid::new_v4()), Some(Uuid::new_v4()), t);
        assert!(matches!(
            next_status(&v, &SettlementEvent::MarkPaid, &actor).unwrap_err(),
            WorkflowError::Terminal(_)
        ));
    }

    #[test]
    fn unrelated_user_cannot_withdraw() {
        let t = Uuid::new_v4();
        let prep = Uuid::new_v4();
        let appr = Uuid::new_v4();
        let stranger = pm(Uuid::new_v4(), t);
        let v = view(SettlementStatus::Approved, Some(prep), Some(appr), t);
        let err = next_status(&v, &SettlementEvent::Withdraw, &stranger).unwrap_err();
        assert!(matches!(err, WorkflowError::NotAuthorizedToWithdraw));
    }

    #[test]
    fn reopen_only_from_terminal() {
        let t = Uuid::new_v4();
        let actor = pm(Uuid::new_v4(), t);
        let v = view(SettlementStatus::Draft, None, None, t);
        assert!(matches!(
            next_status(&v, &SettlementEvent::Reopen, &actor).unwrap_err(),
            WorkflowError::IllegalTransition { .. }
        ));
        let v2 = view(SettlementStatus::Paid, Some(Uuid::new_v4()), Some(Uuid::new_v4()), t);
        assert_eq!(next_status(&v2, &SettlementEvent::Reopen, &actor).unwrap(),
                   SettlementStatus::Reopened);
    }
}
