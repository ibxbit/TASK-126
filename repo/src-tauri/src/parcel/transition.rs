//! Transition orchestration: validate → persist → append history.
//!
//! This module ties the engine, the auth guard, and the repositories
//! together. It is the single code path exercised by the
//! `cmd_transition_parcel` Tauri command — UI buttons all route here.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

use crate::audit::{audit_role_for, AuditLog, AuditWriter, NewAuditLog};
use crate::auth::{self, AuthError, Permission, Principal};
use crate::parcel::machine::{GuardContext, StateMachine, StateMachineError};
use crate::parcel::state::ParcelState;

#[derive(Debug, Error, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TransitionError {
    #[error(transparent)]
    Auth(#[from] AuthError),

    #[error(transparent)]
    Machine(#[from] StateMachineError),

    #[error("parcel not found: {0}")]
    ParcelNotFound(String),

    #[error("the required permission for this transition is not configured")]
    MissingRulePermission,

    #[error("permission code '{0}' is not recognized")]
    UnknownPermission(String),

    #[error("persistence error: {0}")]
    Persistence(String),

    #[error("location must be non-empty")]
    EmptyLocation,
}

/// UI input. Notes may be empty; location is required; occurred_at is
/// in Unix seconds UTC (the UI formats MM/DD/YYYY + 12-hour for
/// display — storage stays canonical).
#[derive(Debug, Clone, Deserialize)]
pub struct TransitionInput {
    pub parcel_id: Uuid,
    pub tenant_id: Uuid,
    pub to_state: ParcelState,
    pub location: String,
    pub notes: Option<String>,
    pub occurred_at_unix: Option<u64>,
}

/// A row appended to `parcel_transitions`. Chain-hashed for tamper
/// evidence: `chain_hash = sha256(prev_chain_hash || canonical_form)`.
#[derive(Debug, Clone, Serialize)]
pub struct TransitionRecord {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub parcel_id: Uuid,
    pub from_state: Option<ParcelState>,
    pub to_state: ParcelState,
    pub operator_user_id: Uuid,
    pub occurred_at_unix: u64,
    pub location: String,
    /// Already-encrypted blob (caller encrypts using `FieldCipher`
    /// with AAD `parcel_transitions.notes_enc:<id>`).
    pub notes_enc: Option<Vec<u8>>,
    pub prev_chain_hash: Option<String>,
    pub chain_hash: String,
}

pub trait ParcelRepository {
    fn current_state(&self, parcel_id: &Uuid) -> Result<Option<ParcelState>, String>;
    fn parcel_tenant(&self, parcel_id: &Uuid) -> Result<Option<Uuid>, String>;
    fn has_check_in_record(&self, parcel_id: &Uuid) -> Result<bool, String>;
    fn update_state(&self, parcel_id: &Uuid, new_state: ParcelState) -> Result<(), String>;
}

pub trait TransitionRepository {
    fn last_chain_hash(&self, parcel_id: &Uuid) -> Result<Option<String>, String>;
    fn append(&self, record: &TransitionRecord) -> Result<(), String>;
    fn history(&self, parcel_id: &Uuid) -> Result<Vec<TransitionRecord>, String>;
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Resolve a permission code string into the typed `Permission` enum.
/// Kept narrow — only parcel-facing permissions are expected here.
fn resolve_permission(code: &str) -> Result<Permission, TransitionError> {
    match code {
        "parcel_operate" => Ok(Permission::ParcelOperate),
        "accept_resident_submission" => Ok(Permission::AcceptResidentSubmission),
        other => Err(TransitionError::UnknownPermission(other.to_string())),
    }
}

fn canonical_bytes(r: &TransitionRecord) -> Vec<u8> {
    // Deterministic byte form — used for chain hashing. Does NOT
    // include notes_enc body (which varies with nonce) but DOES
    // include its sha256 to still tie-in note integrity.
    let notes_digest = r
        .notes_enc
        .as_ref()
        .map(|b| {
            let mut h = Sha256::new();
            h.update(b);
            hex::encode(h.finalize())
        })
        .unwrap_or_default();

    format!(
        "{id}|{tenant}|{parcel}|{from}|{to}|{op}|{ts}|{loc}|{notes}",
        id = r.id,
        tenant = r.tenant_id,
        parcel = r.parcel_id,
        from = r.from_state.map(|s| s.as_str()).unwrap_or("__genesis__"),
        to = r.to_state.as_str(),
        op = r.operator_user_id,
        ts = r.occurred_at_unix,
        loc = r.location,
        notes = notes_digest,
    )
    .into_bytes()
}

fn compute_chain_hash(prev: Option<&str>, record_bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    if let Some(p) = prev {
        h.update(p.as_bytes());
    }
    h.update(record_bytes);
    hex::encode(h.finalize())
}

/// Orchestrate a single transition. All errors roll the caller's
/// transaction back; on success the parcel row is updated, a history
/// record is appended, AND an audit log row is written — all inside
/// one transaction opened by the command handler.
pub fn transition<P: ParcelRepository, T: TransitionRepository, A: AuditWriter>(
    audit: &A,
    principal: &Principal,
    machine: &StateMachine,
    parcels: &P,
    history: &T,
    input: TransitionInput,
    notes_enc: Option<Vec<u8>>,
) -> Result<TransitionRecord, TransitionError> {
    // 1. Input sanity.
    let location = input.location.trim().to_string();
    if location.is_empty() {
        return Err(TransitionError::EmptyLocation);
    }

    // 2. Confirm parcel belongs to this tenant.
    let parcel_tenant = parcels
        .parcel_tenant(&input.parcel_id)
        .map_err(TransitionError::Persistence)?
        .ok_or_else(|| TransitionError::ParcelNotFound(input.parcel_id.to_string()))?;
    if parcel_tenant != input.tenant_id {
        return Err(TransitionError::Auth(AuthError::TenantScopeViolation {
            tenant_id: input.tenant_id.to_string(),
        }));
    }

    // 3. Current state.
    let from = parcels
        .current_state(&input.parcel_id)
        .map_err(TransitionError::Persistence)?;

    // 4. Run state-machine validation (edge exists + guards pass).
    let ctx = GuardContext {
        parcel_id: input.parcel_id,
        tenant_id: input.tenant_id,
        from,
        to: input.to_state,
        has_check_in_record: parcels
            .has_check_in_record(&input.parcel_id)
            .map_err(TransitionError::Persistence)?,
    };
    let rule = machine.apply(&ctx)?;

    // 5. Enforce the rule's required permission (paired with tenant scope).
    let perm_code = rule
        .required_permission
        .as_deref()
        .ok_or(TransitionError::MissingRulePermission)?;
    let perm = resolve_permission(perm_code)?;
    auth::require(principal, perm, &input.tenant_id)?;

    // 6. Build the record and compute the chain hash.
    let occurred_at_unix = input.occurred_at_unix.unwrap_or_else(now_unix);
    let mut record = TransitionRecord {
        id: Uuid::new_v4(),
        tenant_id: input.tenant_id,
        parcel_id: input.parcel_id,
        from_state: from,
        to_state: input.to_state,
        operator_user_id: principal.user_id,
        occurred_at_unix,
        location,
        notes_enc,
        prev_chain_hash: history
            .last_chain_hash(&input.parcel_id)
            .map_err(TransitionError::Persistence)?,
        chain_hash: String::new(),
    };
    record.chain_hash = compute_chain_hash(record.prev_chain_hash.as_deref(), &canonical_bytes(&record));

    // 7. Persist (append-only history + parcel row update).
    history.append(&record).map_err(TransitionError::Persistence)?;
    parcels
        .update_state(&input.parcel_id, input.to_state)
        .map_err(TransitionError::Persistence)?;

    // 8. Audit — same transaction. `from` may be None for genesis.
    let before_state = Some(json!({
        "status": from.map(|s| s.as_str()),
        "has_check_in_record": ctx.has_check_in_record,
    }));
    let after_state = Some(json!({
        "status": input.to_state.as_str(),
    }));
    let audit_log = AuditLog::new(
        NewAuditLog {
            user_id: principal.user_id,
            role: audit_role_for(principal),
            tenant_id: Some(input.tenant_id),
            action_type: "parcel.transition".into(),
            entity_type: "parcel".into(),
            entity_id: Some(input.parcel_id.to_string()),
            before_state,
            after_state,
            metadata: Some(json!({
                "transition_id": record.id,
                "chain_hash": record.chain_hash,
                "location": record.location,
                "session_id": principal.session_id,
            })),
        },
        occurred_at_unix as i64,
    );
    audit
        .append(&audit_log)
        .map_err(TransitionError::Persistence)?;

    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::NoopAuditWriter;
    use crate::auth::context::TenantScope;
    use crate::auth::Role;
    use crate::parcel::machine::{default_guards, GuardCode, TransitionRule};
    use crate::parcel::state::GENESIS;
    use std::cell::RefCell;

    struct MockParcels {
        tenant: Uuid,
        state: RefCell<Option<ParcelState>>,
        has_ci: RefCell<bool>,
    }
    impl ParcelRepository for MockParcels {
        fn current_state(&self, _: &Uuid) -> Result<Option<ParcelState>, String> {
            Ok(*self.state.borrow())
        }
        fn parcel_tenant(&self, _: &Uuid) -> Result<Option<Uuid>, String> {
            Ok(Some(self.tenant))
        }
        fn has_check_in_record(&self, _: &Uuid) -> Result<bool, String> {
            Ok(*self.has_ci.borrow())
        }
        fn update_state(&self, _: &Uuid, s: ParcelState) -> Result<(), String> {
            *self.state.borrow_mut() = Some(s);
            if s == ParcelState::CheckedIn {
                *self.has_ci.borrow_mut() = true;
            }
            Ok(())
        }
    }

    #[derive(Default)]
    struct MockHistory {
        rows: RefCell<Vec<TransitionRecord>>,
    }
    impl TransitionRepository for MockHistory {
        fn last_chain_hash(&self, parcel_id: &Uuid) -> Result<Option<String>, String> {
            Ok(self
                .rows
                .borrow()
                .iter()
                .filter(|r| &r.parcel_id == parcel_id)
                .last()
                .map(|r| r.chain_hash.clone()))
        }
        fn append(&self, r: &TransitionRecord) -> Result<(), String> {
            self.rows.borrow_mut().push(r.clone());
            Ok(())
        }
        fn history(&self, pid: &Uuid) -> Result<Vec<TransitionRecord>, String> {
            Ok(self
                .rows
                .borrow()
                .iter()
                .filter(|r| &r.parcel_id == pid)
                .cloned()
                .collect())
        }
    }

    fn staff(tenant: Uuid) -> Principal {
        Principal::new(
            Uuid::new_v4(),
            "staff".into(),
            Role::Staff,
            TenantScope::single(tenant),
        )
    }

    fn liaison(tenant: Uuid) -> Principal {
        Principal::new(
            Uuid::new_v4(),
            "liaison".into(),
            Role::Liaison,
            TenantScope::single(tenant),
        )
    }

    fn machine() -> StateMachine {
        let rules = vec![
            TransitionRule {
                rule_id: Uuid::new_v4(),
                from_state: GENESIS.into(),
                to_state: ParcelState::CheckedIn,
                guard_code: None,
                required_permission: Some("parcel_operate".into()),
                enabled: true,
            },
            TransitionRule {
                rule_id: Uuid::new_v4(),
                from_state: "checked_in".into(),
                to_state: ParcelState::CheckedOut,
                guard_code: None,
                required_permission: Some("parcel_operate".into()),
                enabled: true,
            },
            TransitionRule {
                rule_id: Uuid::new_v4(),
                from_state: "checked_out".into(),
                to_state: ParcelState::Delivered,
                guard_code: Some(GuardCode::RequiresCheckInExists),
                required_permission: Some("parcel_operate".into()),
                enabled: true,
            },
        ];
        StateMachine::new(rules, default_guards())
    }

    #[test]
    fn happy_path_check_in_out_deliver() {
        let tenant = Uuid::new_v4();
        let sm = machine();
        let pid = Uuid::new_v4();
        let parcels = MockParcels {
            tenant,
            state: RefCell::new(None),
            has_ci: RefCell::new(false),
        };
        let history = MockHistory::default();
        let p = staff(tenant);

        // Genesis → CheckedIn
        transition(&NoopAuditWriter, &p, &sm, &parcels, &history, TransitionInput {
            parcel_id: pid, tenant_id: tenant, to_state: ParcelState::CheckedIn,
            location: "Front Desk".into(), notes: None, occurred_at_unix: Some(1),
        }, None).unwrap();

        // CheckedIn → CheckedOut
        transition(&NoopAuditWriter, &p, &sm, &parcels, &history, TransitionInput {
            parcel_id: pid, tenant_id: tenant, to_state: ParcelState::CheckedOut,
            location: "Locker Bay 2".into(), notes: None, occurred_at_unix: Some(2),
        }, None).unwrap();

        // CheckedOut → Delivered
        transition(&NoopAuditWriter, &p, &sm, &parcels, &history, TransitionInput {
            parcel_id: pid, tenant_id: tenant, to_state: ParcelState::Delivered,
            location: "Locker Bay 2".into(), notes: None, occurred_at_unix: Some(3),
        }, None).unwrap();

        let h = history.history(&pid).unwrap();
        assert_eq!(h.len(), 3);
        // Chain continuity
        assert_eq!(h[1].prev_chain_hash.as_deref(), Some(h[0].chain_hash.as_str()));
        assert_eq!(h[2].prev_chain_hash.as_deref(), Some(h[1].chain_hash.as_str()));
    }

    #[test]
    fn cannot_deliver_without_check_in() {
        let tenant = Uuid::new_v4();
        let sm = machine();
        let pid = Uuid::new_v4();
        let parcels = MockParcels {
            tenant,
            // Pretend someone moved straight to CheckedOut — has_ci stays false.
            state: RefCell::new(Some(ParcelState::CheckedOut)),
            has_ci: RefCell::new(false),
        };
        let history = MockHistory::default();
        let p = staff(tenant);

        let err = transition(&p, &sm, &parcels, &history, TransitionInput {
            parcel_id: pid, tenant_id: tenant, to_state: ParcelState::Delivered,
            location: "Front Desk".into(), notes: None, occurred_at_unix: Some(1),
        }, None).unwrap_err();
        assert!(matches!(err, TransitionError::Machine(StateMachineError::GuardRejected{..})));
    }

    #[test]
    fn liaison_blocked_by_permission() {
        let tenant = Uuid::new_v4();
        let sm = machine();
        let pid = Uuid::new_v4();
        let parcels = MockParcels {
            tenant, state: RefCell::new(None), has_ci: RefCell::new(false),
        };
        let history = MockHistory::default();
        let p = liaison(tenant);

        let err = transition(&p, &sm, &parcels, &history, TransitionInput {
            parcel_id: pid, tenant_id: tenant, to_state: ParcelState::CheckedIn,
            location: "Front Desk".into(), notes: None, occurred_at_unix: Some(1),
        }, None).unwrap_err();
        assert!(matches!(err, TransitionError::Auth(_)));
    }

    #[test]
    fn empty_location_rejected() {
        let tenant = Uuid::new_v4();
        let sm = machine();
        let pid = Uuid::new_v4();
        let parcels = MockParcels {
            tenant, state: RefCell::new(None), has_ci: RefCell::new(false),
        };
        let history = MockHistory::default();
        let p = staff(tenant);
        let err = transition(&p, &sm, &parcels, &history, TransitionInput {
            parcel_id: pid, tenant_id: tenant, to_state: ParcelState::CheckedIn,
            location: "   ".into(), notes: None, occurred_at_unix: Some(1),
        }, None).unwrap_err();
        assert!(matches!(err, TransitionError::EmptyLocation));
    }

    #[test]
    fn cross_tenant_parcel_rejected() {
        let tenant_a = Uuid::new_v4();
        let tenant_b = Uuid::new_v4();
        let sm = machine();
        let pid = Uuid::new_v4();
        let parcels = MockParcels {
            tenant: tenant_a,
            state: RefCell::new(None),
            has_ci: RefCell::new(false),
        };
        let history = MockHistory::default();
        let p = staff(tenant_b);
        let err = transition(&p, &sm, &parcels, &history, TransitionInput {
            parcel_id: pid, tenant_id: tenant_b, to_state: ParcelState::CheckedIn,
            location: "Front Desk".into(), notes: None, occurred_at_unix: Some(1),
        }, None).unwrap_err();
        assert!(matches!(err, TransitionError::Auth(AuthError::TenantScopeViolation{..})));
    }
}
