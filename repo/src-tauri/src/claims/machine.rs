//! Claim lifecycle state machine.
//!
//! Unlike the parcel machine (data-driven rules), the claim lifecycle
//! is policy-critical enough to encode as exhaustive Rust code. Every
//! valid transition is reachable only through a named `ClaimEvent`,
//! which both names the business action and carries its inputs.

use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;
use uuid::Uuid;

use crate::audit::{audit_role_for, AuditLog, AuditWriter, NewAuditLog};
use crate::auth::{self, AuthError, Permission, Principal};
use crate::claims::state::{ClaimStatus, PartyResponse, PartyRole};

/// Claim facts the machine needs to decide a transition. Populated by
/// the repository before a transition is evaluated.
#[derive(Debug, Clone)]
pub struct ClaimView {
    pub claim_id: Uuid,
    pub tenant_id: Uuid,
    pub claimant_user_id: Uuid,
    pub respondent_user_id: Option<Uuid>,
    pub status: ClaimStatus,
    pub reopened_count: u32,
    pub claimant_response: Option<PartyResponse>,
    pub respondent_response: Option<PartyResponse>,
    /// Unix-seconds deadline (72h after submission). Populated when
    /// `status` is `Submitted` or `UnderReview`, otherwise `None`.
    /// Consulted by `timeout::enforce_timeout_lazy` on every access.
    pub response_deadline_unix: Option<i64>,
}

/// Business actions that can be invoked against a claim. Each variant
/// names exactly one transition path.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ClaimEvent {
    Submit,
    Withdraw,
    RespondentEngaged,
    PartyRespond {
        party: PartyRole,
        response: PartyResponse,
    },
    MarkResolved,
    ManagerReject,
    AutoCancel,
    ManagerReopen,
}

#[derive(Debug, Clone, Serialize)]
pub struct TransitionOutcome {
    pub from: ClaimStatus,
    pub to: ClaimStatus,
    pub event: &'static str,
}

#[derive(Debug, Error, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClaimLifecycleError {
    #[error(transparent)]
    Auth(#[from] AuthError),

    #[error("event '{event}' not permitted from status '{status}'")]
    IllegalTransition { event: &'static str, status: String },

    #[error("terminal status '{0}' — use ManagerReopen to continue")]
    TerminalStatus(String),

    #[error("reopen quota exhausted (at most one reopen per claim)")]
    ReopenExhausted,

    #[error("respondent is not assigned to this claim")]
    NoRespondent,

    #[error("only the {expected:?} may perform this action")]
    WrongParty { expected: PartyRole },

    #[error("both parties must respond before {0}")]
    MissingPartyResponse(&'static str),

    #[error("persistence error: {0}")]
    Persistence(String),
}

pub trait ClaimRepository {
    fn load(&self, claim_id: &Uuid) -> Result<Option<ClaimView>, String>;
    /// Update status + mutate related columns (submitted_at,
    /// response_deadline_unix, resolved_at, reopened_count) as a single
    /// row write. The concrete SQLite repo computes the deadline
    /// (now + 72h) when transitioning to Submitted.
    fn apply_status(
        &self,
        claim_id: &Uuid,
        new_status: ClaimStatus,
        event: &str,
        actor: &Principal,
    ) -> Result<(), String>;
    fn record_response(
        &self,
        claim_id: &Uuid,
        party: PartyRole,
        response: PartyResponse,
        actor_user_id: &Uuid,
    ) -> Result<(), String>;
    fn record_reopen(
        &self,
        claim_id: &Uuid,
        requested_by: &Uuid,
        approved_by: &Uuid,
    ) -> Result<(), String>;
}

/// Marker type — namespace for the pure transition table. Kept as a
/// ZST so consumers can depend on "the machine" without a constructor.
pub struct ClaimLifecycleMachine;

impl ClaimLifecycleMachine {
    /// Pure: given the current view and an event, compute the new
    /// status and the permission required. Does NOT touch storage.
    pub fn evaluate(
        view: &ClaimView,
        event: &ClaimEvent,
    ) -> Result<(ClaimStatus, Permission), ClaimLifecycleError> {
        use ClaimEvent::*;
        use ClaimStatus::*;

        // Terminal states are only escapable via ManagerReopen.
        if view.status.is_terminal() && !matches!(event, ManagerReopen) {
            return Err(ClaimLifecycleError::TerminalStatus(
                view.status.as_str().to_string(),
            ));
        }

        let to = match (view.status, event) {
            // Draft → Submitted: claimant files.
            (Draft, Submit) => Submitted,

            // Claimant can withdraw any time before resolution.
            (Draft, Withdraw) | (Submitted, Withdraw) | (UnderReview, Withdraw) => Withdrawn,

            // Respondent engages (implicitly by opening the record).
            (Submitted, RespondentEngaged) => UnderReview,

            // Party responses.
            (Submitted, PartyRespond { party, response })
            | (UnderReview, PartyRespond { party, response }) => {
                decide_after_response(view, *party, *response)?
            }

            // Manager closes a confirmed claim (e.g., after payment).
            (Confirmed, MarkResolved) => Resolved,

            // Manager rejects after contest.
            (Contested, ManagerReject) => RejectedFinal,

            // Contest can also be resolved in favor if manager rules so.
            (Contested, MarkResolved) => Resolved,

            // Timeout driver.
            (Submitted, AutoCancel) | (UnderReview, AutoCancel) => AutoCancelled,

            // Reopen: exactly one, from a terminal state. Gates on
            // `reopened_count` and surfaces `ReopenExhausted` otherwise.
            (_, ManagerReopen) => {
                if view.reopened_count >= 1 {
                    return Err(ClaimLifecycleError::ReopenExhausted);
                }
                Reopened
            }

            // From Reopened a fresh cycle begins — treat like Draft.
            (Reopened, Submit) => Submitted,
            (Reopened, Withdraw) => Withdrawn,

            (status, ev) => {
                return Err(ClaimLifecycleError::IllegalTransition {
                    event: event_name(ev),
                    status: status.as_str().to_string(),
                })
            }
        };

        Ok((to, required_permission(event)))
    }
}

fn decide_after_response(
    view: &ClaimView,
    party: PartyRole,
    response: PartyResponse,
) -> Result<ClaimStatus, ClaimLifecycleError> {
    if view.respondent_user_id.is_none() && party == PartyRole::Respondent {
        return Err(ClaimLifecycleError::NoRespondent);
    }

    // Simulate updated responses.
    let claimant = match party {
        PartyRole::Claimant => Some(response),
        PartyRole::Respondent => view.claimant_response,
    };
    let respondent = match party {
        PartyRole::Respondent => Some(response),
        PartyRole::Claimant => view.respondent_response,
    };

    // Any reject → contested (manager escalation).
    if matches!(claimant, Some(PartyResponse::Reject))
        || matches!(respondent, Some(PartyResponse::Reject))
    {
        return Ok(ClaimStatus::Contested);
    }

    // Both accept → confirmed.
    if matches!(claimant, Some(PartyResponse::Accept))
        && matches!(respondent, Some(PartyResponse::Accept))
    {
        return Ok(ClaimStatus::Confirmed);
    }

    // Still waiting on the other side.
    Ok(ClaimStatus::UnderReview)
}

fn required_permission(event: &ClaimEvent) -> Permission {
    use ClaimEvent::*;
    match event {
        Submit | Withdraw | RespondentEngaged | PartyRespond { .. } => Permission::ViewClaim,
        MarkResolved => Permission::ApproveSettlement,
        ManagerReject => Permission::ApproveSettlement,
        AutoCancel => Permission::ViewClaim, // system-invoked; audited
        ManagerReopen => Permission::ReopenClaim,
    }
}

fn event_name(ev: &ClaimEvent) -> &'static str {
    match ev {
        ClaimEvent::Submit => "submit",
        ClaimEvent::Withdraw => "withdraw",
        ClaimEvent::RespondentEngaged => "respondent_engaged",
        ClaimEvent::PartyRespond { .. } => "party_respond",
        ClaimEvent::MarkResolved => "mark_resolved",
        ClaimEvent::ManagerReject => "manager_reject",
        ClaimEvent::AutoCancel => "auto_cancel",
        ClaimEvent::ManagerReopen => "manager_reopen",
    }
}

/// Orchestration: load → evaluate → auth → persist → audit.
/// All persistence happens in one transaction (opened by the command
/// handler) so the claim update, any party-response / reopen rows,
/// and the audit log entry commit atomically.
pub fn apply_transition<R: ClaimRepository, A: AuditWriter>(
    repo: &R,
    audit: &A,
    principal: &Principal,
    claim_id: &Uuid,
    event: ClaimEvent,
    now_unix: i64,
) -> Result<TransitionOutcome, ClaimLifecycleError> {
    let view = repo
        .load(claim_id)
        .map_err(ClaimLifecycleError::Persistence)?
        .ok_or(ClaimLifecycleError::Persistence(
            "claim not found".to_string(),
        ))?;

    // Party-specific checks for interactive events.
    match &event {
        ClaimEvent::Withdraw => {
            if principal.user_id != view.claimant_user_id {
                return Err(ClaimLifecycleError::WrongParty {
                    expected: PartyRole::Claimant,
                });
            }
        }
        ClaimEvent::PartyRespond { party, .. } => match party {
            PartyRole::Claimant => {
                if principal.user_id != view.claimant_user_id {
                    return Err(ClaimLifecycleError::WrongParty {
                        expected: PartyRole::Claimant,
                    });
                }
            }
            PartyRole::Respondent => match view.respondent_user_id {
                None => return Err(ClaimLifecycleError::NoRespondent),
                Some(r) if r != principal.user_id => {
                    return Err(ClaimLifecycleError::WrongParty {
                        expected: PartyRole::Respondent,
                    })
                }
                _ => {}
            },
        },
        _ => {}
    }

    let (new_status, perm) = ClaimLifecycleMachine::evaluate(&view, &event)?;
    auth::require(principal, perm, &view.tenant_id)?;

    // Persist side-effects specific to some events.
    if let ClaimEvent::PartyRespond { party, response } = &event {
        repo.record_response(claim_id, *party, *response, &principal.user_id)
            .map_err(ClaimLifecycleError::Persistence)?;
    }
    if let ClaimEvent::ManagerReopen = &event {
        repo.record_reopen(claim_id, &view.claimant_user_id, &principal.user_id)
            .map_err(ClaimLifecycleError::Persistence)?;
    }

    repo.apply_status(claim_id, new_status, event_name(&event), principal)
        .map_err(ClaimLifecycleError::Persistence)?;

    // Audit — same transaction as the mutations above.
    let before_state = Some(json!({
        "status": view.status.as_str(),
        "reopened_count": view.reopened_count,
        "claimant_response": view.claimant_response,
        "respondent_response": view.respondent_response,
    }));
    let after_state = Some(json!({
        "status": new_status.as_str(),
    }));
    let mut meta = json!({
        "event": event_name(&event),
        "session_id": principal.session_id,
    });
    if let ClaimEvent::PartyRespond { party, response } = &event {
        meta["party"] = json!(party);
        meta["response"] = json!(response);
    }
    let log = AuditLog::new(
        NewAuditLog {
            user_id: principal.user_id,
            role: audit_role_for(principal),
            tenant_id: Some(view.tenant_id),
            action_type: format!("claim.{}", event_name(&event)),
            entity_type: "claim".into(),
            entity_id: Some(claim_id.to_string()),
            before_state,
            after_state,
            metadata: Some(meta),
        },
        now_unix,
    );
    audit
        .append(&log)
        .map_err(ClaimLifecycleError::Persistence)?;

    Ok(TransitionOutcome {
        from: view.status,
        to: new_status,
        event: event_name(&event),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(status: ClaimStatus) -> ClaimView {
        ClaimView {
            claim_id: Uuid::new_v4(),
            tenant_id: Uuid::new_v4(),
            claimant_user_id: Uuid::new_v4(),
            respondent_user_id: Some(Uuid::new_v4()),
            status,
            reopened_count: 0,
            claimant_response: None,
            respondent_response: None,
            response_deadline_unix: None,
        }
    }

    #[test]
    fn draft_to_submitted() {
        let v = view(ClaimStatus::Draft);
        let (to, _) = ClaimLifecycleMachine::evaluate(&v, &ClaimEvent::Submit).unwrap();
        assert_eq!(to, ClaimStatus::Submitted);
    }

    #[test]
    fn both_accept_confirms() {
        let mut v = view(ClaimStatus::UnderReview);
        v.claimant_response = Some(PartyResponse::Accept);
        let (to, _) = ClaimLifecycleMachine::evaluate(
            &v,
            &ClaimEvent::PartyRespond {
                party: PartyRole::Respondent,
                response: PartyResponse::Accept,
            },
        )
        .unwrap();
        assert_eq!(to, ClaimStatus::Confirmed);
    }

    #[test]
    fn any_reject_contests() {
        let v = view(ClaimStatus::UnderReview);
        let (to, _) = ClaimLifecycleMachine::evaluate(
            &v,
            &ClaimEvent::PartyRespond {
                party: PartyRole::Claimant,
                response: PartyResponse::Reject,
            },
        )
        .unwrap();
        assert_eq!(to, ClaimStatus::Contested);
    }

    #[test]
    fn submitted_auto_cancel() {
        let v = view(ClaimStatus::Submitted);
        let (to, _) = ClaimLifecycleMachine::evaluate(&v, &ClaimEvent::AutoCancel).unwrap();
        assert_eq!(to, ClaimStatus::AutoCancelled);
    }

    #[test]
    fn terminal_blocks_everything_but_reopen() {
        let v = view(ClaimStatus::Resolved);
        let err = ClaimLifecycleMachine::evaluate(&v, &ClaimEvent::Submit).unwrap_err();
        assert!(matches!(err, ClaimLifecycleError::TerminalStatus(_)));

        let (to, _) = ClaimLifecycleMachine::evaluate(&v, &ClaimEvent::ManagerReopen).unwrap();
        assert_eq!(to, ClaimStatus::Reopened);
    }

    #[test]
    fn reopen_quota_enforced() {
        let mut v = view(ClaimStatus::Resolved);
        v.reopened_count = 1;
        let err = ClaimLifecycleMachine::evaluate(&v, &ClaimEvent::ManagerReopen).unwrap_err();
        assert!(matches!(err, ClaimLifecycleError::ReopenExhausted));
    }

    #[test]
    fn illegal_transition_surfaces() {
        let v = view(ClaimStatus::Draft);
        let err = ClaimLifecycleMachine::evaluate(&v, &ClaimEvent::MarkResolved).unwrap_err();
        assert!(matches!(err, ClaimLifecycleError::IllegalTransition { .. }));
    }
}
