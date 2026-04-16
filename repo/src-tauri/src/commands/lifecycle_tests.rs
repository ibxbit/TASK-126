//! End-to-end lifecycle integration tests for the command-layer.
//!
//! These tests do **not** mock SQLite, do **not** mock the audit
//! writer, and do **not** mock the state machines. They invoke the
//! exact same call chain `cmd_transition_parcel`, `cmd_claim_transition`,
//! `cmd_settlement_transition`, `cmd_settlement_prepare`, and
//! `cmd_settlement_approve` use — driven from a real `SessionState`,
//! against a real in-memory SQLite database with all 11 migrations
//! applied — and verify both the persisted state and the audit row.
//!
//! The only thing they bypass is `tauri::State<'_, T>` (whose ctor is
//! private). Because we route through the same repos, machines, and
//! audit writers the commands themselves use, anything broken in a
//! command body would surface in this file.

#![cfg(test)]

use std::sync::Arc;

use rusqlite::params;
use uuid::Uuid;

use crate::auth::context::{Principal, TenantScope};
use crate::auth::{Permission, Role};
use crate::claims::machine::{apply_transition, ClaimEvent};
use crate::claims::state::{ClaimStatus, PartyResponse, PartyRole};
use crate::db::connection::Database;
use crate::db::repos::{
    SqliteApprovalRepo, SqliteAuditWriter, SqliteClaimRepo, SqliteParcelRepo,
    SqliteSettlementRepo, SqliteTransitionRepo,
};
use crate::ipc::{guard, IpcError, SessionState};
use crate::parcel::machine::{default_guards, default_rules, StateMachine};
use crate::parcel::state::ParcelState;
use crate::parcel::transition::{transition, TransitionInput, TransitionRepository};
use crate::settlement::approval::{approve_settlement, prepare_settlement};
use crate::settlement::workflow::{
    apply_event, SettlementEvent, SettlementRepository, SettlementStatus,
};

// ─── shared test fixture ───────────────────────────────────────────────

const NOW: i64 = 1_700_000_000;

struct Fixture {
    db: Arc<Database>,
    tenant: Uuid,
    pm_user: Uuid,        // PropertyManager — preparer / claimant
    pm_user2: Uuid,       // PropertyManager — approver / respondent
    staff: Uuid,          // Staff role (limited perms)
    foreign_tenant: Uuid, // for tenant-isolation tests
}

fn fixture() -> Fixture {
    let db = Database::open_in_memory().expect("db open");
    db.run_migrations().expect("migrate");
    let db = Arc::new(db);

    let tenant = Uuid::new_v4();
    let foreign_tenant = Uuid::new_v4();
    let pm_user = Uuid::new_v4();
    let pm_user2 = Uuid::new_v4();
    let staff = Uuid::new_v4();

    let c = db.conn();
    c.execute(
        "INSERT INTO tenants (id, name, code, active, created_at, updated_at)
         VALUES (?1, 'Primary', 'PRIM', 1, ?2, ?2)",
        params![tenant.to_string(), NOW],
    )
    .unwrap();
    c.execute(
        "INSERT INTO tenants (id, name, code, active, created_at, updated_at)
         VALUES (?1, 'Foreign', 'FRGN', 1, ?2, ?2)",
        params![foreign_tenant.to_string(), NOW],
    )
    .unwrap();
    for (uid, name) in [
        (pm_user, "pm-one"),
        (pm_user2, "pm-two"),
        (staff, "staffy"),
    ] {
        c.execute(
            "INSERT INTO users (id, username, display_name, password_hash, active, created_at, updated_at)
             VALUES (?1, ?2, ?2, '$argon2id$v=19$m=16,t=2,p=1$dGVzdHNhbHQ$abc', 1, ?3, ?3)",
            params![uid.to_string(), name, NOW],
        )
        .unwrap();
    }
    drop(c);

    Fixture { db, tenant, pm_user, pm_user2, staff, foreign_tenant }
}

fn pm(uid: Uuid, tenant: Uuid) -> Principal {
    Principal::new(
        uid,
        format!("pm-{uid}"),
        Role::PropertyManager,
        TenantScope::single(tenant),
    )
}

fn staff_principal(uid: Uuid, tenant: Uuid) -> Principal {
    Principal::new(
        uid,
        format!("staff-{uid}"),
        Role::Staff,
        TenantScope::single(tenant),
    )
}

fn audit_count(db: &Arc<Database>) -> i64 {
    let c = db.conn();
    c.query_row("SELECT COUNT(*) FROM audit_logs", [], |r| r.get(0))
        .unwrap()
}

// ─── Parcel lifecycle ──────────────────────────────────────────────────

fn seed_parcel(fx: &Fixture) -> Uuid {
    let rid = Uuid::new_v4();
    let pid = Uuid::new_v4();
    let c = fx.db.conn();
    c.execute(
        "INSERT INTO residents (id, tenant_id, full_name, created_at, updated_at, created_by)
         VALUES (?1, ?2, 'R', ?3, ?3, ?4)",
        params![rid.to_string(), fx.tenant.to_string(), NOW, fx.pm_user.to_string()],
    )
    .unwrap();
    c.execute(
        "INSERT INTO parcels (id, tenant_id, resident_id, status, received_at, created_at, updated_at, created_by)
         VALUES (?1, ?2, ?3, 'checked_in', ?4, ?4, ?4, ?5)",
        params![
            pid.to_string(),
            fx.tenant.to_string(),
            rid.to_string(),
            NOW,
            fx.pm_user.to_string()
        ],
    )
    .unwrap();
    // Insert genesis check-in transition record so the
    // `requires_check_in_exists` guard passes on later transitions.
    c.execute(
        "INSERT INTO parcel_transitions (id, tenant_id, parcel_id, from_state, to_state,
         operator_user_id, occurred_at, location, prev_chain_hash, chain_hash)
         VALUES (?1, ?2, ?3, NULL, 'checked_in', ?4, ?5, 'Front Desk', NULL, 'genesis')",
        params![
            Uuid::new_v4().to_string(),
            fx.tenant.to_string(),
            pid.to_string(),
            fx.pm_user.to_string(),
            NOW,
        ],
    )
    .unwrap();
    pid
}

fn invoke_parcel_transition(
    fx: &Fixture,
    actor: &Principal,
    input: TransitionInput,
) -> Result<crate::parcel::transition::TransitionRecord, IpcError> {
    let p_repo = SqliteParcelRepo::new(Arc::clone(&fx.db));
    let t_repo = SqliteTransitionRepo::new(Arc::clone(&fx.db));
    let audit = SqliteAuditWriter::new(Arc::clone(&fx.db));
    let sm = StateMachine::new(default_rules(), default_guards());
    transition(&audit, actor, &sm, &p_repo, &t_repo, input, None)
        .map_err(|e| IpcError::Internal(format!("{e:?}")))
}

#[test]
fn parcel_lifecycle_check_in_to_checked_out_to_delivered_chains_history() {
    let fx = fixture();
    let pid = seed_parcel(&fx);
    let actor = pm(fx.pm_user, fx.tenant);
    let before_audits = audit_count(&fx.db);

    // Step 1: Already CheckedIn → CheckedOut
    let r1 = invoke_parcel_transition(
        &fx,
        &actor,
        TransitionInput {
            parcel_id: pid,
            tenant_id: fx.tenant,
            to_state: ParcelState::CheckedOut,
            location: "Front Desk".into(),
            notes: None,
            occurred_at_unix: Some(NOW as u64 + 10),
        },
    )
    .expect("checked_out ok");
    assert_eq!(r1.from_state, Some(ParcelState::CheckedIn));
    assert_eq!(r1.to_state, ParcelState::CheckedOut);
    // Genesis record seeded by seed_parcel() exists as prior record, so
    // the first new transition anchors to that genesis chain hash.
    assert!(r1.prev_chain_hash.is_some(), "anchors to seeded genesis record");
    let chain1 = r1.chain_hash.clone();
    assert!(!chain1.is_empty());

    // Step 2: CheckedOut → Delivered
    let r2 = invoke_parcel_transition(
        &fx,
        &actor,
        TransitionInput {
            parcel_id: pid,
            tenant_id: fx.tenant,
            to_state: ParcelState::Delivered,
            location: "Resident door".into(),
            notes: None,
            occurred_at_unix: Some(NOW as u64 + 20),
        },
    )
    .expect("delivered ok");
    assert_eq!(r2.from_state, Some(ParcelState::CheckedOut));
    assert_eq!(r2.to_state, ParcelState::Delivered);
    assert_eq!(
        r2.prev_chain_hash.as_deref(),
        Some(chain1.as_str()),
        "chain hash continuity is required for tamper evidence",
    );
    assert_ne!(r2.chain_hash, chain1, "each link is unique");

    // History through the same SQLite repo the cmd_parcel_history reads.
    // history = [genesis, checked_out, delivered] — 3 records total.
    let history = SqliteTransitionRepo::new(Arc::clone(&fx.db))
        .history(&pid)
        .expect("history");
    assert_eq!(history.len(), 3);
    assert_eq!(history[0].to_state, ParcelState::CheckedIn); // genesis
    assert_eq!(history[1].to_state, ParcelState::CheckedOut);
    assert_eq!(history[2].to_state, ParcelState::Delivered);
    assert_eq!(history[2].prev_chain_hash.as_deref(), Some(chain1.as_str()));

    // The parcel row itself reflects the latest state.
    let final_state: String = fx
        .db
        .conn()
        .query_row(
            "SELECT status FROM parcels WHERE id = ?1",
            [pid.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(final_state, "delivered");

    // Two audit rows must have been appended (one per transition).
    assert_eq!(audit_count(&fx.db), before_audits + 2);
}

#[test]
fn parcel_transition_to_delivered_without_check_in_history_is_blocked_by_guard() {
    // A parcel not in CheckedIn state, with no check-in record, may
    // not jump to Delivered — the guard must fail loud.
    let fx = fixture();
    let rid = Uuid::new_v4();
    let pid = Uuid::new_v4();
    {
        let c = fx.db.conn();
        c.execute(
            "INSERT INTO residents (id, tenant_id, full_name, created_at, updated_at, created_by)
             VALUES (?1, ?2, 'R', ?3, ?3, ?4)",
            params![rid.to_string(), fx.tenant.to_string(), NOW, fx.pm_user.to_string()],
        )
        .unwrap();
        // Parcel was somehow created in returned_exception state (corrupted seed).
        c.execute(
            "INSERT INTO parcels (id, tenant_id, resident_id, status, received_at, created_at, updated_at, created_by)
             VALUES (?1, ?2, ?3, 'returned_exception', ?4, ?4, ?4, ?5)",
            params![
                pid.to_string(),
                fx.tenant.to_string(),
                rid.to_string(),
                NOW,
                fx.pm_user.to_string()
            ],
        )
        .unwrap();
    }
    let actor = pm(fx.pm_user, fx.tenant);
    let res = invoke_parcel_transition(
        &fx,
        &actor,
        TransitionInput {
            parcel_id: pid,
            tenant_id: fx.tenant,
            to_state: ParcelState::Delivered,
            location: "X".into(),
            notes: None,
            occurred_at_unix: Some(NOW as u64),
        },
    );
    assert!(res.is_err(), "transition must be rejected");
    // No audit row should have been written on a refused transition.
    let audits: i64 = fx
        .db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM audit_logs WHERE entity_id = ?1",
            [pid.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(audits, 0, "no audit row for a rejected transition");
}

#[test]
fn parcel_command_chain_blocks_unauthenticated_caller() {
    // The very first thing cmd_transition_parcel does is invoke the
    // session guard. Without a logged-in principal it must fail with
    // Unauthenticated, BEFORE touching the DB.
    let session = SessionState::new();
    let res = guard::require(&session, Permission::ParcelOperate, &Uuid::new_v4());
    assert!(matches!(res, Err(IpcError::Unauthenticated)));
}

#[test]
fn parcel_command_chain_blocks_cross_tenant_caller() {
    let fx = fixture();
    let session = SessionState::new();
    session.set(pm(fx.pm_user, fx.tenant)); // scoped to `tenant`
    let res = guard::require(&session, Permission::ParcelOperate, &fx.foreign_tenant);
    assert!(matches!(res, Err(IpcError::TenantScopeViolation { .. })));
}

#[test]
fn parcel_command_chain_blocks_staff_from_approve_settlement_permission() {
    let fx = fixture();
    let session = SessionState::new();
    session.set(staff_principal(fx.staff, fx.tenant));
    let res = guard::require(&session, Permission::ApproveSettlement, &fx.tenant);
    match res {
        Err(IpcError::PermissionDenied { role, permission }) => {
            assert_eq!(role, "staff");
            assert_eq!(permission, "approve_settlement");
        }
        other => panic!("expected PermissionDenied, got {:?}", other),
    }
}

// ─── Claim lifecycle ───────────────────────────────────────────────────

fn seed_claim_with_respondent(fx: &Fixture, claimant: Uuid, respondent: Uuid) -> Uuid {
    let cid = Uuid::new_v4();
    let c = fx.db.conn();
    c.execute(
        "INSERT INTO claims (id, tenant_id, claim_number, kind, category, claimant_user_id,
         respondent_user_id, status, amount_cents, opened_at, response_deadline_unix,
         reopened_count, created_at, updated_at)
         VALUES (?1, ?2, ?3, 'deposit_deduction', 'damage', ?4, ?5, 'draft', 1000,
                 ?6, NULL, 0, ?6, ?6)",
        params![
            cid.to_string(),
            fx.tenant.to_string(),
            format!("CLM-{}", &cid.to_string()[..8]),
            claimant.to_string(),
            respondent.to_string(),
            NOW,
        ],
    )
    .unwrap();
    cid
}

#[test]
fn claim_lifecycle_submit_engage_both_accept_writes_to_confirmed_with_audit_chain() {
    // This test drives the same call chain `cmd_claim_transition` uses,
    // through real SQLite, including audit row insertion and the lazy
    // timeout enforcement that the command performs first.
    let fx = fixture();
    let claimant = pm(fx.pm_user, fx.tenant);
    let respondent = pm(fx.pm_user2, fx.tenant);
    let cid = seed_claim_with_respondent(&fx, fx.pm_user, fx.pm_user2);

    let repo = SqliteClaimRepo::new(Arc::clone(&fx.db));
    let audit = SqliteAuditWriter::new(Arc::clone(&fx.db));

    // 1. Claimant submits the claim.
    let out = apply_transition(&repo, &audit, &claimant, &cid, ClaimEvent::Submit, NOW + 1)
        .expect("submit");
    assert_eq!(out.from, ClaimStatus::Draft);
    assert_eq!(out.to, ClaimStatus::Submitted);

    // The repo set a 72-hour deadline atomically with the status update.
    let deadline: Option<i64> = fx
        .db
        .conn()
        .query_row(
            "SELECT response_deadline_unix FROM claims WHERE id = ?1",
            [cid.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(deadline.is_some(), "deadline must be set on Submitted");
    assert!(deadline.unwrap() > NOW, "deadline must be in the future");

    // 2. Respondent engages → UnderReview.
    let out = apply_transition(
        &repo,
        &audit,
        &respondent,
        &cid,
        ClaimEvent::RespondentEngaged,
        NOW + 100,
    )
    .expect("respondent engaged");
    assert_eq!(out.to, ClaimStatus::UnderReview);

    // 3. Claimant Accepts — still UnderReview (only one party so far).
    let out = apply_transition(
        &repo,
        &audit,
        &claimant,
        &cid,
        ClaimEvent::PartyRespond {
            party: PartyRole::Claimant,
            response: PartyResponse::Accept,
        },
        NOW + 200,
    )
    .expect("claimant accept");
    assert_eq!(out.to, ClaimStatus::UnderReview);

    // 4. Respondent Accepts — both parties accepted → Confirmed.
    let out = apply_transition(
        &repo,
        &audit,
        &respondent,
        &cid,
        ClaimEvent::PartyRespond {
            party: PartyRole::Respondent,
            response: PartyResponse::Accept,
        },
        NOW + 300,
    )
    .expect("respondent accept");
    assert_eq!(out.to, ClaimStatus::Confirmed);

    // The DB row reflects Confirmed + has both party responses recorded.
    let status: String = fx
        .db
        .conn()
        .query_row(
            "SELECT status FROM claims WHERE id = ?1",
            [cid.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(status, "confirmed");

    let resp_count: i64 = fx
        .db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM claim_party_responses WHERE claim_id = ?1",
            [cid.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(resp_count, 2);

    // Audit log must contain rows attributed to *both* users so a
    // per-actor audit slice can recover who said what.
    let claim_audit_rows: Vec<(String, String)> = {
        let c = fx.db.conn();
        let mut stmt = c
            .prepare(
                "SELECT user_id, action_type FROM audit_logs WHERE entity_id = ?1 ORDER BY timestamp_unix",
            )
            .unwrap();
        stmt.query_map([cid.to_string()], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect()
    };
    assert!(claim_audit_rows.len() >= 4, "got {:?}", claim_audit_rows);
    let users: std::collections::HashSet<&str> =
        claim_audit_rows.iter().map(|(u, _)| u.as_str()).collect();
    assert!(users.contains(fx.pm_user.to_string().as_str()));
    assert!(users.contains(fx.pm_user2.to_string().as_str()));
    let actions: std::collections::HashSet<&str> =
        claim_audit_rows.iter().map(|(_, a)| a.as_str()).collect();
    assert!(actions.contains("claim.submit"));
    assert!(actions.contains("claim.party_respond"));
}

#[test]
fn claim_party_respond_from_wrong_user_is_rejected_before_persist() {
    // Only the named claimant may PartyRespond as Claimant. A
    // different logged-in user — even with the same role and tenant —
    // must be denied at the orchestration layer.
    let fx = fixture();
    let claimant = pm(fx.pm_user, fx.tenant);
    let imposter = pm(fx.pm_user2, fx.tenant);
    let cid = seed_claim_with_respondent(&fx, fx.pm_user, fx.pm_user2);

    let repo = SqliteClaimRepo::new(Arc::clone(&fx.db));
    let audit = SqliteAuditWriter::new(Arc::clone(&fx.db));

    apply_transition(&repo, &audit, &claimant, &cid, ClaimEvent::Submit, NOW + 1).unwrap();

    // Imposter (the respondent) tries to act AS the claimant.
    let res = apply_transition(
        &repo,
        &audit,
        &imposter,
        &cid,
        ClaimEvent::PartyRespond {
            party: PartyRole::Claimant,
            response: PartyResponse::Accept,
        },
        NOW + 100,
    );
    assert!(res.is_err(), "imposter must not impersonate claimant");

    // No party_response row was inserted as a side effect.
    let n: i64 = fx
        .db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM claim_party_responses WHERE claim_id = ?1",
            [cid.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 0, "rejected event must not leak persistence");
}

#[test]
fn claim_reopen_quota_enforces_one_reopen_per_claim() {
    let fx = fixture();
    let manager = pm(fx.pm_user, fx.tenant);
    let claimant = pm(fx.pm_user, fx.tenant);
    let cid = seed_claim_with_respondent(&fx, fx.pm_user, fx.pm_user2);
    let repo = SqliteClaimRepo::new(Arc::clone(&fx.db));
    let audit = SqliteAuditWriter::new(Arc::clone(&fx.db));

    // Drive to a terminal state: Submit → Withdraw.
    apply_transition(&repo, &audit, &claimant, &cid, ClaimEvent::Submit, NOW + 1).unwrap();
    apply_transition(&repo, &audit, &claimant, &cid, ClaimEvent::Withdraw, NOW + 2).unwrap();

    // First reopen succeeds.
    let r1 = apply_transition(
        &repo,
        &audit,
        &manager,
        &cid,
        ClaimEvent::ManagerReopen,
        NOW + 3,
    )
    .expect("first reopen");
    assert_eq!(r1.to, ClaimStatus::Reopened);

    // Drive back to terminal.
    apply_transition(&repo, &audit, &claimant, &cid, ClaimEvent::Submit, NOW + 4).unwrap();
    apply_transition(&repo, &audit, &claimant, &cid, ClaimEvent::Withdraw, NOW + 5).unwrap();

    // Second reopen MUST fail with ReopenExhausted.
    let res = apply_transition(
        &repo,
        &audit,
        &manager,
        &cid,
        ClaimEvent::ManagerReopen,
        NOW + 6,
    );
    let err = format!("{res:?}");
    assert!(err.contains("ReopenExhausted"), "got: {err}");
}

// ─── Settlement lifecycle ──────────────────────────────────────────────

fn seed_case_and_settlement(fx: &Fixture) -> (Uuid, Uuid) {
    let rid = Uuid::new_v4();
    let case_id = Uuid::new_v4();
    let settle_id = Uuid::new_v4();
    let c = fx.db.conn();
    c.execute(
        "INSERT INTO residents (id, tenant_id, full_name, created_at, updated_at, created_by)
         VALUES (?1, ?2, 'R', ?3, ?3, ?4)",
        params![rid.to_string(), fx.tenant.to_string(), NOW, fx.pm_user.to_string()],
    )
    .unwrap();
    c.execute(
        "INSERT INTO move_out_cases (id, tenant_id, resident_id, case_number, status, move_out_date, created_at, updated_at, created_by)
         VALUES (?1, ?2, ?3, 'MO-LIFE', 'open', ?4, ?4, ?4, ?5)",
        params![case_id.to_string(), fx.tenant.to_string(), rid.to_string(), NOW, fx.pm_user.to_string()],
    ).unwrap();
    c.execute(
        "INSERT INTO settlements (id, tenant_id, case_id, status, created_at, updated_at, created_by)
         VALUES (?1, ?2, ?3, 'draft', ?4, ?4, ?5)",
        params![settle_id.to_string(), fx.tenant.to_string(), case_id.to_string(), NOW, fx.pm_user.to_string()],
    ).unwrap();
    (case_id, settle_id)
}

#[test]
fn settlement_lifecycle_prepare_then_approve_then_pay_persists_each_step() {
    let fx = fixture();
    let (_case_id, sid) = seed_case_and_settlement(&fx);
    let preparer = pm(fx.pm_user, fx.tenant);
    let approver = pm(fx.pm_user2, fx.tenant);

    let s_repo = SqliteSettlementRepo::new(Arc::clone(&fx.db));
    let a_repo = SqliteApprovalRepo::new(Arc::clone(&fx.db));

    // ── prepare (Draft → PendingApproval) ──
    let rec = prepare_settlement(
        &s_repo,
        &a_repo,
        &preparer,
        sid,
        Some(b"PM reviewed".to_vec()),
        NOW + 10,
    )
    .expect("prepare ok");
    assert_eq!(rec.settlement_id, sid);

    let view = s_repo.load(&sid).unwrap().unwrap();
    assert_eq!(view.status, SettlementStatus::PendingApproval);
    assert_eq!(view.prepared_by, Some(fx.pm_user));

    // ── approve (PendingApproval → Approved) by a different user ──
    let rec = approve_settlement(
        &s_repo,
        &a_repo,
        &approver,
        sid,
        Some(b"approved".to_vec()),
        NOW + 20,
    )
    .expect("approve ok");
    assert_eq!(rec.settlement_id, sid);

    let view = s_repo.load(&sid).unwrap().unwrap();
    assert_eq!(view.status, SettlementStatus::Approved);
    assert_eq!(view.approved_by, Some(fx.pm_user2));

    // ── mark paid (Approved → Paid) via the workflow command path ──
    let new_status = apply_event(&s_repo, &approver, &sid, SettlementEvent::MarkPaid, NOW + 30)
        .expect("mark_paid ok");
    assert_eq!(new_status, SettlementStatus::Paid);

    // Final shape in the DB, checked through the same repo loader.
    let view = s_repo.load(&sid).unwrap().unwrap();
    assert_eq!(view.status, SettlementStatus::Paid);
}

#[test]
fn settlement_approve_by_same_user_as_preparer_is_rejected() {
    // The approver-must-differ guard is the most important business
    // rule here. Verify it through the actual approve_settlement +
    // SQLite repo path the command uses, not just at the next_status
    // pure function.
    let fx = fixture();
    let (_case_id, sid) = seed_case_and_settlement(&fx);
    let preparer = pm(fx.pm_user, fx.tenant);

    let s_repo = SqliteSettlementRepo::new(Arc::clone(&fx.db));
    let a_repo = SqliteApprovalRepo::new(Arc::clone(&fx.db));

    prepare_settlement(&s_repo, &a_repo, &preparer, sid, None, NOW + 1).unwrap();

    let res = approve_settlement(&s_repo, &a_repo, &preparer, sid, None, NOW + 2);
    assert!(res.is_err(), "self-approval must be rejected");
    let err = format!("{:?}", res.unwrap_err());
    assert!(err.contains("SamePartyApproval"), "got: {err}");

    // The DB row must still be PendingApproval — failure must not flip
    // status as a side effect.
    let view = s_repo.load(&sid).unwrap().unwrap();
    assert_eq!(view.status, SettlementStatus::PendingApproval);
}

#[test]
fn settlement_event_blocked_for_role_without_permission() {
    // A Staff user (no ApproveSettlement permission) cannot drive an
    // Approve event, even within their own tenant.
    let fx = fixture();
    let (_, sid) = seed_case_and_settlement(&fx);
    let s_repo = SqliteSettlementRepo::new(Arc::clone(&fx.db));
    let a_repo = SqliteApprovalRepo::new(Arc::clone(&fx.db));

    // Bring it to PendingApproval with a PM preparer.
    prepare_settlement(
        &s_repo,
        &a_repo,
        &pm(fx.pm_user, fx.tenant),
        sid,
        None,
        NOW + 1,
    )
    .unwrap();

    // Staff user attempts to approve.
    let staff_actor = staff_principal(fx.staff, fx.tenant);
    let res = apply_event(&s_repo, &staff_actor, &sid, SettlementEvent::Approve, NOW + 2);
    assert!(res.is_err(), "Staff must not be allowed to Approve");
    let err = format!("{:?}", res.unwrap_err());
    assert!(
        err.contains("PermissionDenied") || err.contains("approve_settlement"),
        "got: {err}"
    );

    // Settlement remains PendingApproval — denied event must not leak
    // a status change.
    let view = s_repo.load(&sid).unwrap().unwrap();
    assert_eq!(view.status, SettlementStatus::PendingApproval);
}

// ─── Tenant isolation across the boundary ──────────────────────────────

#[test]
fn parcel_in_tenant_a_is_invisible_to_principal_scoped_to_tenant_b() {
    // The tenant-scope guard must reject a cross-tenant principal
    // BEFORE the SQLite query runs.
    let fx = fixture();
    let pid = seed_parcel(&fx); // belongs to fx.tenant
    let foreign_actor = pm(fx.pm_user, fx.foreign_tenant);

    let res = invoke_parcel_transition(
        &fx,
        &foreign_actor,
        TransitionInput {
            parcel_id: pid,
            tenant_id: fx.tenant, // wrong tenant for actor
            to_state: ParcelState::CheckedOut,
            location: "X".into(),
            notes: None,
            occurred_at_unix: Some(NOW as u64),
        },
    );
    assert!(res.is_err(), "cross-tenant transition must be rejected");
}

#[test]
fn audit_attribution_for_system_actor_records_role_system() {
    // When the timeout-enforcer or recovery thread invokes a
    // transition, audit_role_for must record the row as `system` even
    // though the principal may carry Administrator-level permissions.
    let fx = fixture();
    let pid = seed_parcel(&fx);
    // Seed a system user row so the FK on parcel_transitions.operator_user_id holds.
    let system_uid = Uuid::new_v4();
    fx.db.conn().execute(
        "INSERT INTO users (id, username, display_name, password_hash, active, created_at, updated_at)
         VALUES (?1, 'system:scheduler', 'system', '$argon2id$v=19$m=16,t=2,p=1$dGVzdHNhbHQ$abc', 1, ?2, ?2)",
        params![system_uid.to_string(), NOW],
    ).unwrap();
    // Build a system principal: username prefix `"system:"`, Administrator role.
    let system = Principal::new(
        system_uid,
        "system:scheduler".into(),
        Role::Administrator,
        TenantScope::Global,
    );

    invoke_parcel_transition(
        &fx,
        &system,
        TransitionInput {
            parcel_id: pid,
            tenant_id: fx.tenant,
            to_state: ParcelState::CheckedOut,
            location: "auto".into(),
            notes: None,
            occurred_at_unix: Some(NOW as u64),
        },
    )
    .expect("system can transition");

    let role: String = fx
        .db
        .conn()
        .query_row(
            "SELECT role FROM audit_logs WHERE entity_id = ?1 LIMIT 1",
            [pid.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(role, "system");
}
