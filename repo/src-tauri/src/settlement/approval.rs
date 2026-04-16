//! Two-step approval signatures.
//!
//! `prepare_settlement` records the first signature and drives the
//! workflow into PendingApproval; `approve_settlement` records the
//! second, distinct signature and drives it into Approved. Each
//! signature row is immutable (DB triggers enforce this) and the
//! UNIQUE(settlement_id, step) constraint prevents double-signing.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::auth::{AuthError, Principal};
use crate::settlement::workflow::{
    apply_event, SettlementEvent, SettlementRepository, SettlementStatus, WorkflowError,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStep {
    Prepared,
    Approved,
}

impl ApprovalStep {
    pub fn as_str(&self) -> &'static str {
        match self {
            ApprovalStep::Prepared => "prepared",
            ApprovalStep::Approved => "approved",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ApprovalRecord {
    pub id: Uuid,
    pub settlement_id: Uuid,
    pub step: ApprovalStep,
    pub user_id: Uuid,
    pub signed_at: i64,
}

#[derive(Debug, Error, Serialize)]
#[serde(tag = "type", content = "detail", rename_all = "snake_case")]
pub enum ApprovalError {
    #[error(transparent)]
    Auth(#[from] AuthError),

    #[error(transparent)]
    Workflow(#[from] WorkflowError),

    #[error("step '{0}' has already been signed for this settlement")]
    AlreadySigned(&'static str),

    #[error("persistence error: {0}")]
    Persistence(String),
}

pub trait ApprovalRepository {
    /// Insert a signature row. Must fail (returning Err) if a row with
    /// the same (settlement_id, step) already exists — the schema
    /// enforces this with UNIQUE.
    fn insert(
        &self,
        record: &ApprovalRecord,
        notes_enc: Option<&[u8]>,
    ) -> Result<(), String>;

    fn fetch(
        &self,
        settlement_id: &Uuid,
        step: ApprovalStep,
    ) -> Result<Option<ApprovalRecord>, String>;
}

/// First sign-off. Locks line items by transitioning the settlement
/// to PendingApproval. The DB-level update trigger on
/// `settlement_approvals` makes this signature permanent.
pub fn prepare_settlement<S: SettlementRepository, A: ApprovalRepository>(
    settlement_repo: &S,
    approval_repo: &A,
    principal: &Principal,
    settlement_id: Uuid,
    notes_enc: Option<Vec<u8>>,
    now_unix: i64,
) -> Result<ApprovalRecord, ApprovalError> {
    if approval_repo
        .fetch(&settlement_id, ApprovalStep::Prepared)
        .map_err(ApprovalError::Persistence)?
        .is_some()
    {
        return Err(ApprovalError::AlreadySigned("prepared"));
    }

    let record = ApprovalRecord {
        id: Uuid::new_v4(),
        settlement_id,
        step: ApprovalStep::Prepared,
        user_id: principal.user_id,
        signed_at: now_unix,
    };

    // Persist signature first so a workflow failure does not leave
    // a hanging "prepared" status without its signature row. If the
    // workflow transition fails the caller's transaction rolls both
    // back together.
    approval_repo
        .insert(&record, notes_enc.as_deref())
        .map_err(ApprovalError::Persistence)?;

    apply_event(
        settlement_repo,
        principal,
        &settlement_id,
        SettlementEvent::Prepare,
        now_unix,
    )?;

    Ok(record)
}

/// Second sign-off. Must be a different user than the preparer; the
/// workflow rejects same-party approval before the row is inserted.
pub fn approve_settlement<S: SettlementRepository, A: ApprovalRepository>(
    settlement_repo: &S,
    approval_repo: &A,
    principal: &Principal,
    settlement_id: Uuid,
    notes_enc: Option<Vec<u8>>,
    now_unix: i64,
) -> Result<ApprovalRecord, ApprovalError> {
    if approval_repo
        .fetch(&settlement_id, ApprovalStep::Approved)
        .map_err(ApprovalError::Persistence)?
        .is_some()
    {
        return Err(ApprovalError::AlreadySigned("approved"));
    }

    // Run the workflow first — it enforces SamePartyApproval.
    let new_status = apply_event(
        settlement_repo,
        principal,
        &settlement_id,
        SettlementEvent::Approve,
        now_unix,
    )?;
    debug_assert_eq!(new_status, SettlementStatus::Approved);

    let record = ApprovalRecord {
        id: Uuid::new_v4(),
        settlement_id,
        step: ApprovalStep::Approved,
        user_id: principal.user_id,
        signed_at: now_unix,
    };
    approval_repo
        .insert(&record, notes_enc.as_deref())
        .map_err(ApprovalError::Persistence)?;

    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::context::TenantScope;
    use crate::auth::Role;
    use crate::settlement::workflow::SettlementView;
    use std::cell::RefCell;
    use std::collections::HashMap;

    struct MockSettlements {
        view: RefCell<SettlementView>,
    }
    impl SettlementRepository for MockSettlements {
        fn load(&self, _: &Uuid) -> Result<Option<SettlementView>, String> {
            Ok(Some(self.view.borrow().clone()))
        }
        fn set_status(
            &self,
            _id: &Uuid,
            new_status: SettlementStatus,
            actor: &Principal,
            _now: i64,
        ) -> Result<(), String> {
            let mut v = self.view.borrow_mut();
            v.status = new_status;
            if new_status == SettlementStatus::PendingApproval {
                v.prepared_by = Some(actor.user_id);
            }
            if new_status == SettlementStatus::Approved {
                v.approved_by = Some(actor.user_id);
            }
            Ok(())
        }
    }

    #[derive(Default)]
    struct MockApprovals {
        rows: RefCell<HashMap<(Uuid, &'static str), ApprovalRecord>>,
    }
    impl ApprovalRepository for MockApprovals {
        fn insert(&self, r: &ApprovalRecord, _n: Option<&[u8]>) -> Result<(), String> {
            let key = (r.settlement_id, r.step.as_str());
            if self.rows.borrow().contains_key(&key) {
                return Err("UNIQUE violation".into());
            }
            self.rows.borrow_mut().insert(key, r.clone());
            Ok(())
        }
        fn fetch(&self, sid: &Uuid, step: ApprovalStep) -> Result<Option<ApprovalRecord>, String> {
            Ok(self.rows.borrow().get(&(*sid, step.as_str())).cloned())
        }
    }

    fn pm(uid: Uuid, tenant: Uuid) -> Principal {
        Principal::new(uid, "pm".into(), Role::PropertyManager, TenantScope::single(tenant))
    }

    #[test]
    fn happy_two_step_approval() {
        let tenant = Uuid::new_v4();
        let sid = Uuid::new_v4();
        let s_repo = MockSettlements {
            view: RefCell::new(SettlementView {
                settlement_id: sid,
                tenant_id: tenant,
                status: SettlementStatus::Draft,
                prepared_by: None,
                approved_by: None,
            }),
        };
        let a_repo = MockApprovals::default();
        let preparer = pm(Uuid::new_v4(), tenant);
        let approver = pm(Uuid::new_v4(), tenant);

        prepare_settlement(&s_repo, &a_repo, &preparer, sid, None, 100).unwrap();
        approve_settlement(&s_repo, &a_repo, &approver, sid, None, 200).unwrap();

        assert_eq!(s_repo.view.borrow().status, SettlementStatus::Approved);
        assert_eq!(s_repo.view.borrow().prepared_by, Some(preparer.user_id));
        assert_eq!(s_repo.view.borrow().approved_by, Some(approver.user_id));
    }

    #[test]
    fn same_party_cannot_self_approve() {
        let tenant = Uuid::new_v4();
        let sid = Uuid::new_v4();
        let s_repo = MockSettlements {
            view: RefCell::new(SettlementView {
                settlement_id: sid, tenant_id: tenant,
                status: SettlementStatus::Draft, prepared_by: None, approved_by: None,
            }),
        };
        let a_repo = MockApprovals::default();
        let user = pm(Uuid::new_v4(), tenant);

        prepare_settlement(&s_repo, &a_repo, &user, sid, None, 100).unwrap();
        let err = approve_settlement(&s_repo, &a_repo, &user, sid, None, 200).unwrap_err();
        assert!(matches!(err, ApprovalError::Workflow(WorkflowError::SamePartyApproval)));
    }

    #[test]
    fn double_prepare_blocked() {
        let tenant = Uuid::new_v4();
        let sid = Uuid::new_v4();
        let s_repo = MockSettlements {
            view: RefCell::new(SettlementView {
                settlement_id: sid, tenant_id: tenant,
                status: SettlementStatus::Draft, prepared_by: None, approved_by: None,
            }),
        };
        let a_repo = MockApprovals::default();
        let preparer = pm(Uuid::new_v4(), tenant);

        prepare_settlement(&s_repo, &a_repo, &preparer, sid, None, 100).unwrap();
        let err = prepare_settlement(&s_repo, &a_repo, &preparer, sid, None, 101).unwrap_err();
        assert!(matches!(err, ApprovalError::AlreadySigned("prepared")));
    }
}
