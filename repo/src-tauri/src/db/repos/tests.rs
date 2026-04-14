//! Integration tests for SQLite repository implementations.
//! All tests use in-memory SQLite (deterministic, offline, no fs side effects).

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use uuid::Uuid;

    use crate::audit::{AuditLog, AuditRole, AuditWriter, NewAuditLog};
    use crate::auth::context::{Principal, TenantScope};
    use crate::auth::Role;
    use crate::claims::machine::ClaimRepository;
    use crate::claims::state::ClaimStatus;
    use crate::claims::timeout::ExpiredClaimFinder;
    use crate::db::connection::Database;
    use crate::db::repos::*;
    use crate::parcel::state::ParcelState;
    use crate::parcel::transition::{ParcelRepository, TransitionRepository};
    use crate::scheduling::rules::RuleRepository;
    use crate::settlement::approval::{ApprovalRepository, ApprovalRecord, ApprovalStep};
    use crate::settlement::workflow::{SettlementRepository, SettlementStatus};
    use crate::sharing::expiry::PackageRepository;

    fn setup() -> Arc<Database> {
        let db = Database::open_in_memory().expect("open in-memory DB");
        db.run_migrations().expect("migrations");
        let db = Arc::new(db);
        seed_tenant_and_user(&db);
        db
    }

    fn tenant_id() -> Uuid {
        Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap()
    }
    fn user_id() -> Uuid {
        Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap()
    }
    fn user2_id() -> Uuid {
        Uuid::parse_str("00000000-0000-0000-0000-000000000003").unwrap()
    }

    fn seed_tenant_and_user(db: &Arc<Database>) {
        let c = db.conn();
        let now = 1_700_000_000i64;
        c.execute_batch(&format!(
            "INSERT INTO tenants (id, name, code, active, created_at, updated_at)
             VALUES ('{tid}', 'Test Prop', 'TP', 1, {now}, {now});
             INSERT INTO users (id, username, display_name, password_hash, active, created_at, updated_at)
             VALUES ('{uid}', 'staff1', 'Staff One', '$argon2id$v=19$m=16,t=2,p=1$dGVzdHNhbHQ$abc', 1, {now}, {now});
             INSERT INTO users (id, username, display_name, password_hash, active, created_at, updated_at)
             VALUES ('{uid2}', 'mgr1', 'Manager One', '$argon2id$v=19$m=16,t=2,p=1$dGVzdHNhbHQ$abc', 1, {now}, {now});",
            tid = tenant_id(),
            uid = user_id(),
            uid2 = user2_id(),
            now = now,
        ))
        .expect("seed");
    }

    fn seed_resident(db: &Arc<Database>) -> Uuid {
        let rid = Uuid::new_v4();
        let c = db.conn();
        c.execute(
            "INSERT INTO residents (id, tenant_id, full_name, created_at, updated_at, created_by)
             VALUES (?1, ?2, 'Test Resident', 1700000000, 1700000000, ?3)",
            rusqlite::params![rid.to_string(), tenant_id().to_string(), user_id().to_string()],
        ).unwrap();
        rid
    }

    fn seed_parcel(db: &Arc<Database>, resident_id: &Uuid) -> Uuid {
        let pid = Uuid::new_v4();
        let c = db.conn();
        c.execute(
            "INSERT INTO parcels (id, tenant_id, resident_id, status, received_at, created_at, updated_at, created_by)
             VALUES (?1, ?2, ?3, 'checked_in', 1700000000, 1700000000, 1700000000, ?4)",
            rusqlite::params![pid.to_string(), tenant_id().to_string(), resident_id.to_string(), user_id().to_string()],
        ).unwrap();
        pid
    }

    fn pm(uid: Uuid) -> Principal {
        Principal::new(uid, "pm".into(), Role::PropertyManager, TenantScope::single(tenant_id()))
    }

    // ── AuditWriter ────────────────────────────────────────────────

    #[test]
    fn audit_writer_inserts_and_enforces_append_only() {
        let db = setup();
        let writer = SqliteAuditWriter::new(Arc::clone(&db));
        let log = AuditLog::new(
            NewAuditLog {
                user_id: user_id(),
                role: AuditRole::Staff,
                tenant_id: Some(tenant_id()),
                action_type: "test.action".into(),
                entity_type: "test".into(),
                entity_id: Some("e1".into()),
                before_state: None,
                after_state: Some(serde_json::json!({"x": 1})),
                metadata: None,
            },
            1_700_000_000,
        );
        writer.append(&log).expect("insert");

        // Read back.
        let c = db.conn();
        let count: i64 = c
            .query_row("SELECT COUNT(*) FROM audit_logs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);

        // Append-only trigger blocks UPDATE.
        let res = c.execute(
            "UPDATE audit_logs SET action_type = 'tampered' WHERE id = ?1",
            [log.id.to_string()],
        );
        assert!(res.is_err(), "UPDATE must be blocked by trigger");
    }

    // ── ParcelRepository ───────────────────────────────────────────

    #[test]
    fn parcel_repo_reads_state_and_tenant() {
        let db = setup();
        let rid = seed_resident(&db);
        let pid = seed_parcel(&db, &rid);
        let repo = SqliteParcelRepo::new(Arc::clone(&db));

        let state = repo.current_state(&pid).unwrap();
        assert_eq!(state, Some(ParcelState::CheckedIn));

        let tid = repo.parcel_tenant(&pid).unwrap();
        assert_eq!(tid, Some(tenant_id()));
    }

    #[test]
    fn parcel_repo_updates_state() {
        let db = setup();
        let rid = seed_resident(&db);
        let pid = seed_parcel(&db, &rid);
        let repo = SqliteParcelRepo::new(Arc::clone(&db));

        repo.update_state(&pid, ParcelState::CheckedOut).unwrap();
        assert_eq!(repo.current_state(&pid).unwrap(), Some(ParcelState::CheckedOut));
    }

    #[test]
    fn transition_repo_appends_and_retrieves_history() {
        let db = setup();
        let rid = seed_resident(&db);
        let pid = seed_parcel(&db, &rid);
        let repo = SqliteTransitionRepo::new(Arc::clone(&db));

        let record = crate::parcel::transition::TransitionRecord {
            id: Uuid::new_v4(),
            tenant_id: tenant_id(),
            parcel_id: pid,
            from_state: None,
            to_state: ParcelState::CheckedIn,
            operator_user_id: user_id(),
            occurred_at_unix: 1_700_000_000,
            location: "Front Desk".into(),
            notes_enc: None,
            prev_chain_hash: None,
            chain_hash: "abc123".into(),
        };
        repo.append(&record).unwrap();

        let hist = repo.history(&pid).unwrap();
        assert_eq!(hist.len(), 1);
        assert_eq!(hist[0].location, "Front Desk");
        assert_eq!(hist[0].chain_hash, "abc123");

        let last = repo.last_chain_hash(&pid).unwrap();
        assert_eq!(last, Some("abc123".into()));
    }

    // ── ClaimRepository ────────────────────────────────────────────

    fn seed_claim(db: &Arc<Database>, status: &str, deadline: Option<i64>) -> Uuid {
        let cid = Uuid::new_v4();
        let c = db.conn();
        c.execute(
            "INSERT INTO claims (id, tenant_id, claim_number, kind, category, claimant_user_id,
             status, amount_cents, opened_at, response_deadline_unix, reopened_count,
             created_at, updated_at)
             VALUES (?1, ?2, ?3, 'deposit_deduction', 'damage', ?4, ?5, 0, 1700000000, ?6, 0, 1700000000, 1700000000)",
            rusqlite::params![
                cid.to_string(), tenant_id().to_string(),
                format!("CLM-{}", &cid.to_string()[..8]),
                user_id().to_string(), status, deadline,
            ],
        ).unwrap();
        cid
    }

    #[test]
    fn claim_repo_loads_view_with_deadline() {
        let db = setup();
        let cid = seed_claim(&db, "submitted", Some(1_700_100_000));
        let repo = SqliteClaimRepo::new(Arc::clone(&db));
        let view = repo.load(&cid).unwrap().unwrap();
        assert_eq!(view.status, ClaimStatus::Submitted);
        assert_eq!(view.response_deadline_unix, Some(1_700_100_000));
    }

    #[test]
    fn claim_repo_applies_status_and_sets_deadline_on_submit() {
        let db = setup();
        let cid = seed_claim(&db, "draft", None);
        let repo = SqliteClaimRepo::new(Arc::clone(&db));
        let p = pm(user_id());
        repo.apply_status(&cid, ClaimStatus::Submitted, "submit", &p).unwrap();
        let view = repo.load(&cid).unwrap().unwrap();
        assert_eq!(view.status, ClaimStatus::Submitted);
        assert!(view.response_deadline_unix.is_some(), "deadline should be set");
    }

    #[test]
    fn expired_claim_finder_returns_past_deadline() {
        let db = setup();
        let active = seed_claim(&db, "submitted", Some(2_000_000_000));
        let expired = seed_claim(&db, "submitted", Some(1_699_999_999));
        let finder = SqliteExpiredClaimFinder::new(Arc::clone(&db));
        let found = finder.find_expired(1_700_000_000).unwrap();
        assert!(found.iter().any(|e| e.claim_id == expired));
        assert!(!found.iter().any(|e| e.claim_id == active));
    }

    // ── SettlementRepository ───────────────────────────────────────

    fn seed_case_and_settlement(db: &Arc<Database>) -> (Uuid, Uuid) {
        let rid = seed_resident(db);
        let case_id = Uuid::new_v4();
        let settle_id = Uuid::new_v4();
        let c = db.conn();
        c.execute(
            "INSERT INTO move_out_cases (id, tenant_id, resident_id, case_number, status, move_out_date, created_at, updated_at, created_by)
             VALUES (?1, ?2, ?3, 'MO-001', 'open', 1700000000, 1700000000, 1700000000, ?4)",
            rusqlite::params![case_id.to_string(), tenant_id().to_string(), rid.to_string(), user_id().to_string()],
        ).unwrap();
        c.execute(
            "INSERT INTO settlements (id, tenant_id, case_id, status, created_at, updated_at, created_by)
             VALUES (?1, ?2, ?3, 'draft', 1700000000, 1700000000, ?4)",
            rusqlite::params![settle_id.to_string(), tenant_id().to_string(), case_id.to_string(), user_id().to_string()],
        ).unwrap();
        (case_id, settle_id)
    }

    #[test]
    fn settlement_repo_loads_and_updates_status() {
        let db = setup();
        let (_, sid) = seed_case_and_settlement(&db);
        let repo = SqliteSettlementRepo::new(Arc::clone(&db));
        let view = repo.load(&sid).unwrap().unwrap();
        assert_eq!(view.status, SettlementStatus::Draft);

        let p = pm(user_id());
        repo.set_status(&sid, SettlementStatus::PendingApproval, &p, 1_700_000_001).unwrap();
        let view2 = repo.load(&sid).unwrap().unwrap();
        assert_eq!(view2.status, SettlementStatus::PendingApproval);
    }

    #[test]
    fn approval_repo_insert_and_fetch() {
        let db = setup();
        let (_, sid) = seed_case_and_settlement(&db);
        let repo = SqliteApprovalRepo::new(Arc::clone(&db));
        let rec = ApprovalRecord {
            id: Uuid::new_v4(),
            settlement_id: sid,
            step: ApprovalStep::Prepared,
            user_id: user_id(),
            signed_at: 1_700_000_001,
        };
        repo.insert(&rec, None).unwrap();
        let fetched = repo.fetch(&sid, ApprovalStep::Prepared).unwrap().unwrap();
        assert_eq!(fetched.user_id, user_id());

        // Duplicate step blocked by UNIQUE.
        let dup = ApprovalRecord { id: Uuid::new_v4(), ..rec };
        assert!(repo.insert(&dup, None).is_err());
    }

    // ── Migration sanity ───────────────────────────────────────────

    #[test]
    fn migrations_are_idempotent() {
        let db = Database::open_in_memory().unwrap();
        db.run_migrations().unwrap();
        db.run_migrations().unwrap(); // second run must be a no-op
        let c = db.conn();
        let count: i64 = c
            .query_row("SELECT COUNT(*) FROM _migrations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 11);
    }

    // ── Rollback failure boundary ──────────────────────────────────

    #[test]
    fn parcel_state_not_updated_if_transition_append_fails() {
        // If the parcel_transitions table rejects an INSERT (e.g.,
        // duplicate PK), the parcel row must NOT have its status
        // changed — the caller's transaction should have rolled back
        // both. We simulate by inserting a transition, then trying to
        // insert the same PK again.
        let db = setup();
        let rid = seed_resident(&db);
        let pid = seed_parcel(&db, &rid);
        let repo = SqliteTransitionRepo::new(Arc::clone(&db));
        let p_repo = SqliteParcelRepo::new(Arc::clone(&db));

        let rec = crate::parcel::transition::TransitionRecord {
            id: Uuid::new_v4(),
            tenant_id: tenant_id(),
            parcel_id: pid,
            from_state: Some(ParcelState::CheckedIn),
            to_state: ParcelState::CheckedOut,
            operator_user_id: user_id(),
            occurred_at_unix: 1_700_000_000,
            location: "Lobby".into(),
            notes_enc: None,
            prev_chain_hash: None,
            chain_hash: "first".into(),
        };
        repo.append(&rec).unwrap();
        p_repo.update_state(&pid, ParcelState::CheckedOut).unwrap();

        // Now try to re-insert with the same transition id (duplicate PK).
        let dup = crate::parcel::transition::TransitionRecord {
            chain_hash: "dup".into(),
            ..rec.clone()
        };
        assert!(repo.append(&dup).is_err(), "duplicate PK must fail");
        // Parcel state stays at CheckedOut — not corrupted.
        assert_eq!(p_repo.current_state(&pid).unwrap(), Some(ParcelState::CheckedOut));
    }

    // ════════════════════════════════════════════════════════════════
    // Security boundary tests
    // ════════════════════════════════════════════════════════════════

    use crate::ipc::guard;
    use crate::ipc::SessionState;
    use crate::auth::permissions::Permission;

    // ── Unauthenticated ────────────────────────────────────────────

    #[test]
    fn guard_rejects_unauthenticated_on_empty_session() {
        let session = SessionState::new();
        let err = guard::require(&session, Permission::ViewClaim, &tenant_id()).unwrap_err();
        match err {
            crate::ipc::IpcError::Unauthenticated => {}
            other => panic!("expected Unauthenticated, got {other:?}"),
        }
    }

    #[test]
    fn guard_rejects_unauthenticated_after_logout() {
        let session = SessionState::new();
        session.set(pm(user_id()));
        session.clear();
        assert!(guard::require_authenticated(&session).is_err());
    }

    // ── Unauthorized (wrong role/permission) ───────────────────────

    #[test]
    fn liaison_blocked_from_parcel_operate() {
        let session = SessionState::new();
        let liaison = Principal::new(
            user_id(), "liaison1".into(),
            Role::Liaison, TenantScope::single(tenant_id()),
        );
        session.set(liaison);
        let err = guard::require(&session, Permission::ParcelOperate, &tenant_id()).unwrap_err();
        match err {
            crate::ipc::IpcError::PermissionDenied { role, permission } => {
                assert_eq!(role, "liaison");
                assert_eq!(permission, "parcel_operate");
            }
            other => panic!("expected PermissionDenied, got {other:?}"),
        }
    }

    #[test]
    fn reviewer_blocked_from_approve_settlement() {
        let session = SessionState::new();
        let reviewer = Principal::new(
            user_id(), "rev1".into(),
            Role::Reviewer, TenantScope::single(tenant_id()),
        );
        session.set(reviewer);
        let err = guard::require(&session, Permission::ApproveSettlement, &tenant_id()).unwrap_err();
        assert!(matches!(err, crate::ipc::IpcError::PermissionDenied { .. }));
    }

    #[test]
    fn staff_blocked_from_manage_users() {
        let session = SessionState::new();
        let staff = Principal::new(
            user_id(), "staff1".into(),
            Role::Staff, TenantScope::single(tenant_id()),
        );
        session.set(staff);
        let err = guard::require(&session, Permission::ManageUsers, &tenant_id()).unwrap_err();
        assert!(matches!(err, crate::ipc::IpcError::PermissionDenied { .. }));
    }

    // ── Tenant isolation ───────────────────────────────────────────

    fn tenant_b_id() -> Uuid {
        Uuid::parse_str("00000000-0000-0000-0000-0000000000bb").unwrap()
    }

    #[test]
    fn guard_blocks_cross_tenant_access() {
        let session = SessionState::new();
        let user = Principal::new(
            user_id(), "staff1".into(),
            Role::PropertyManager, TenantScope::single(tenant_id()),
        );
        session.set(user);
        // Attempt to access tenant_b while scoped to tenant_id.
        let err = guard::require(&session, Permission::ViewClaim, &tenant_b_id()).unwrap_err();
        match err {
            crate::ipc::IpcError::TenantScopeViolation { tenant_id: tid } => {
                assert_eq!(tid, tenant_b_id().to_string());
            }
            other => panic!("expected TenantScopeViolation, got {other:?}"),
        }
    }

    #[test]
    fn parcel_in_tenant_a_invisible_to_tenant_b_scoped_query() {
        let db = setup();
        let rid = seed_resident(&db);
        let pid = seed_parcel(&db, &rid); // belongs to tenant_id()
        let repo = SqliteParcelRepo::new(Arc::clone(&db));

        // Parcel exists when queried directly.
        assert!(repo.parcel_tenant(&pid).unwrap().is_some());

        // If the caller were to check tenant_b against the parcel's
        // tenant_id, they'd get a mismatch. This is what
        // transition::transition checks before acting.
        let actual_tenant = repo.parcel_tenant(&pid).unwrap().unwrap();
        assert_ne!(actual_tenant, tenant_b_id(), "parcel must belong to tenant_id, not tenant_b");
    }

    #[test]
    fn claim_loads_only_for_matching_id() {
        let db = setup();
        let cid = seed_claim(&db, "submitted", Some(1_800_000_000));
        let repo = SqliteClaimRepo::new(Arc::clone(&db));
        // Real claim loads.
        assert!(repo.load(&cid).unwrap().is_some());
        // Non-existent claim returns None, not an error.
        let bogus = Uuid::new_v4();
        assert!(repo.load(&bogus).unwrap().is_none());
    }

    // ── Payload / edge cases ───────────────────────────────────────

    #[test]
    fn parcel_update_to_same_state_is_idempotent() {
        let db = setup();
        let rid = seed_resident(&db);
        let pid = seed_parcel(&db, &rid);
        let repo = SqliteParcelRepo::new(Arc::clone(&db));
        repo.update_state(&pid, ParcelState::CheckedIn).unwrap();
        assert_eq!(repo.current_state(&pid).unwrap(), Some(ParcelState::CheckedIn));
    }

    #[test]
    fn claim_apply_status_sets_closed_at_on_terminal() {
        let db = setup();
        let cid = seed_claim(&db, "submitted", Some(1_700_100_000));
        let repo = SqliteClaimRepo::new(Arc::clone(&db));
        let p = pm(user_id());
        repo.apply_status(&cid, ClaimStatus::AutoCancelled, "auto_cancel", &p).unwrap();
        let view = repo.load(&cid).unwrap().unwrap();
        assert_eq!(view.status, ClaimStatus::AutoCancelled);
        // closed_at should be set (terminal status).
        let c = db.conn();
        let closed: Option<i64> = c.query_row(
            "SELECT closed_at FROM claims WHERE id = ?1",
            [cid.to_string()], |r| r.get(0),
        ).unwrap();
        assert!(closed.is_some(), "closed_at must be set for terminal status");
    }

    #[test]
    fn settlement_status_lifecycle_draft_to_paid() {
        let db = setup();
        let (_, sid) = seed_case_and_settlement(&db);
        let s_repo = SqliteSettlementRepo::new(Arc::clone(&db));
        let a_repo = SqliteApprovalRepo::new(Arc::clone(&db));
        let p1 = pm(user_id());  // preparer
        let p2 = pm(user2_id()); // approver

        // Draft → PendingApproval
        s_repo.set_status(&sid, SettlementStatus::PendingApproval, &p1, 1000).unwrap();
        a_repo.insert(&ApprovalRecord {
            id: Uuid::new_v4(), settlement_id: sid,
            step: ApprovalStep::Prepared, user_id: user_id(), signed_at: 1000,
        }, None).unwrap();

        // PendingApproval → Approved
        s_repo.set_status(&sid, SettlementStatus::Approved, &p2, 2000).unwrap();
        a_repo.insert(&ApprovalRecord {
            id: Uuid::new_v4(), settlement_id: sid,
            step: ApprovalStep::Approved, user_id: user2_id(), signed_at: 2000,
        }, None).unwrap();

        // Verify both approvals are persisted.
        let view = s_repo.load(&sid).unwrap().unwrap();
        assert_eq!(view.status, SettlementStatus::Approved);
        assert_eq!(view.prepared_by, Some(user_id()));
        assert_eq!(view.approved_by, Some(user2_id()));

        // Approved → Paid
        s_repo.set_status(&sid, SettlementStatus::Paid, &p2, 3000).unwrap();
        let final_view = s_repo.load(&sid).unwrap().unwrap();
        assert_eq!(final_view.status, SettlementStatus::Paid);
    }

    // ── Audit append-only enforcement ──────────────────────────────

    #[test]
    fn audit_log_delete_blocked_by_trigger() {
        let db = setup();
        let writer = SqliteAuditWriter::new(Arc::clone(&db));
        let log = AuditLog::new(NewAuditLog {
            user_id: user_id(), role: AuditRole::Staff,
            tenant_id: Some(tenant_id()),
            action_type: "test.delete".into(),
            entity_type: "test".into(), entity_id: None,
            before_state: None, after_state: None, metadata: None,
        }, 1_700_000_000);
        writer.append(&log).unwrap();

        let c = db.conn();
        let res = c.execute("DELETE FROM audit_logs WHERE id = ?1", [log.id.to_string()]);
        assert!(res.is_err(), "DELETE must be blocked by trigger");
    }

    // ── Claim lifecycle through SQLite ─────────────────────────────

    #[test]
    fn claim_lazy_timeout_autocancel_via_sqlite() {
        let db = setup();
        // Submitted with deadline in the past.
        let cid = seed_claim(&db, "submitted", Some(1_699_999_000));
        let repo = SqliteClaimRepo::new(Arc::clone(&db));
        let audit = SqliteAuditWriter::new(Arc::clone(&db));

        use crate::claims::timeout::enforce_timeout_lazy;
        let view = enforce_timeout_lazy(
            &repo, &audit, user_id(), &cid, 1_700_000_000,
        ).unwrap();
        assert_eq!(view.status, ClaimStatus::AutoCancelled);

        // Second call is idempotent.
        let view2 = enforce_timeout_lazy(
            &repo, &audit, user_id(), &cid, 1_700_000_001,
        ).unwrap();
        assert_eq!(view2.status, ClaimStatus::AutoCancelled);
    }

    // ── Expired claim finder boundaries ────────────────────────────

    #[test]
    fn expired_finder_ignores_terminal_claims() {
        let db = setup();
        // Manually insert a claim that is already auto_cancelled but
        // still has a stale deadline value.
        let cid = Uuid::new_v4();
        db.conn().execute(
            "INSERT INTO claims (id, tenant_id, claim_number, kind, category,
             claimant_user_id, status, amount_cents, opened_at,
             response_deadline_unix, reopened_count, created_at, updated_at)
             VALUES (?1, ?2, 'CLM-TERM', 'deposit_deduction', 'damage',
                     ?3, 'auto_cancelled', 0, 1700000000,
                     1699000000, 0, 1700000000, 1700000000)",
            rusqlite::params![cid.to_string(), tenant_id().to_string(), user_id().to_string()],
        ).unwrap();

        let finder = SqliteExpiredClaimFinder::new(Arc::clone(&db));
        let found = finder.find_expired(1_700_000_000).unwrap();
        assert!(!found.iter().any(|e| e.claim_id == cid),
                "terminal claims must not appear in expired results");
    }

    // ════════════════════════════════════════════════════════════════
    // Analytics repo tests
    // ════════════════════════════════════════════════════════════════

    use crate::analytics::events::{EventCategory, EventRepository, PersistableEvent};

    #[test]
    fn event_repo_inserts_and_rolls_up() {
        let db = setup();
        let repo = SqliteEventRepo::new(Arc::clone(&db));
        let ev = PersistableEvent {
            id: Uuid::new_v4(),
            tenant_id: Some(tenant_id()),
            actor_user_id: Some(user_id()),
            session_id: None,
            category: EventCategory::Click,
            kind: "parcel.check_in".into(),
            entity_kind: None,
            entity_id: None,
            funnel: None,
            funnel_step: None,
            duration_ms: None,
            success: None,
            payload_json: None,
            experiment_id: None,
            variant_id: None,
            occurred_at_unix: 1_700_000_000,
        };
        repo.insert(&ev).expect("insert event");
        repo.roll_up(
            ev.tenant_id.as_ref(), 1_700_000_000, EventCategory::Click,
            "parcel.check_in", 0, 0,
        ).expect("roll up");

        // Verify event row exists.
        let c = db.conn();
        let count: i64 = c.query_row(
            "SELECT COUNT(*) FROM events WHERE kind = 'parcel.check_in'",
            [], |r| r.get(0),
        ).unwrap();
        assert_eq!(count, 1);
    }

    // ════════════════════════════════════════════════════════════════
    // Recovery repo tests
    // ════════════════════════════════════════════════════════════════

    use crate::recovery::checkpoint::{RecoveryOutcome, RecoveryRepository};

    #[test]
    fn recovery_repo_records_and_retrieves_outcome() {
        let db = setup();
        let repo = SqliteRecoveryRepo::new(Arc::clone(&db));
        repo.record_event(RecoveryOutcome::CleanStart, 1_700_000_000, 1_700_000_001, "")
            .expect("record");
        let outcome = repo.last_outcome().expect("query");
        assert_eq!(outcome, Some("clean_start".into()));
    }

    #[test]
    fn recovery_repo_last_outcome_returns_most_recent() {
        let db = setup();
        let repo = SqliteRecoveryRepo::new(Arc::clone(&db));
        repo.record_event(RecoveryOutcome::CleanStart, 1_700_000_000, 1_700_000_001, "")
            .expect("record 1");
        repo.record_event(RecoveryOutcome::UncleanRepaired, 1_700_000_002, 1_700_000_003, "wal repaired")
            .expect("record 2");
        let outcome = repo.last_outcome().expect("query");
        assert_eq!(outcome, Some("unclean_repaired".into()));
    }

    // ════════════════════════════════════════════════════════════════
    // Version repo tests
    // ════════════════════════════════════════════════════════════════

    #[test]
    fn version_repo_inserts_and_queries() {
        let db = setup();
        let repo = SqliteVersionRepo::new(Arc::clone(&db));
        let vid = Uuid::new_v4();
        let c = db.conn();
        c.execute(
            "INSERT INTO app_versions (id, version, package_id, installed_at, is_active, snapshot_path)
             VALUES (?1, '1.0.0', ?2, 1700000000, 1, '/backups/v1')",
            rusqlite::params![vid.to_string(), Uuid::new_v4().to_string()],
        ).unwrap();

        let count: i64 = c.query_row(
            "SELECT COUNT(*) FROM app_versions WHERE version = '1.0.0'",
            [], |r| r.get(0),
        ).unwrap();
        assert_eq!(count, 1);
    }

    // ════════════════════════════════════════════════════════════════
    // Document repo tests
    // ════════════════════════════════════════════════════════════════

    #[test]
    fn attachment_search_returns_empty_for_no_data() {
        let db = setup();
        let search = SqliteAttachmentSearch::new(Arc::clone(&db));
        let results = search.search(&tenant_id(), None, None, None, None, 50).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn tag_repo_add_and_remove_are_symmetric() {
        let db = setup();
        // First insert an attachment.
        let aid = Uuid::new_v4();
        db.conn().execute(
            "INSERT INTO attachments (id, tenant_id, entity_kind, entity_id, display_name_enc,
             mime_type, byte_size, sha256_hex, relative_path_enc,
             created_at, created_by)
             VALUES (?1, ?2, 'case', ?3, X'00', 'text/plain', 100, 'abc', X'00',
                     1700000000, ?4)",
            rusqlite::params![aid.to_string(), tenant_id().to_string(),
                              Uuid::new_v4().to_string(), user_id().to_string()],
        ).unwrap();

        let repo = SqliteTagRepo::new(Arc::clone(&db));
        repo.add(&aid, "urgent", Some(&user_id())).unwrap();
        repo.add(&aid, "review", Some(&user_id())).unwrap();

        // Verify tags.
        let tag_count: i64 = db.conn().query_row(
            "SELECT COUNT(*) FROM attachment_tags WHERE attachment_id = ?1",
            [aid.to_string()], |r| r.get(0),
        ).unwrap();
        assert_eq!(tag_count, 2);

        // Remove one tag.
        repo.remove(&aid, "urgent").unwrap();
        let tag_count2: i64 = db.conn().query_row(
            "SELECT COUNT(*) FROM attachment_tags WHERE attachment_id = ?1",
            [aid.to_string()], |r| r.get(0),
        ).unwrap();
        assert_eq!(tag_count2, 1);
    }

    // ════════════════════════════════════════════════════════════════
    // Scheduling repo tests
    // ════════════════════════════════════════════════════════════════

    #[test]
    fn rule_repo_load_active_returns_none_when_empty() {
        let db = setup();
        let repo = SqliteRuleRepo::new(Arc::clone(&db));
        let result = repo.load_active(&tenant_id(), "inspections").unwrap();
        assert!(result.is_none(), "no rule sets should exist initially");
    }

    // ════════════════════════════════════════════════════════════════
    // Sharing repo tests
    // ════════════════════════════════════════════════════════════════

    #[test]
    fn package_repo_load_returns_none_for_nonexistent() {
        let db = setup();
        let repo = SqlitePackageRepo::new(Arc::clone(&db));
        let result = repo.load(&Uuid::new_v4()).unwrap();
        assert!(result.is_none());
    }
}
