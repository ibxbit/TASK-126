# Coverage Mapping — Requirement → Command → Test

## Parcel Lifecycle

| Requirement | Backend command | Service function | Test coverage |
|---|---|---|---|
| Check-in / check-out / deliver parcels | `cmd_transition_parcel` | `parcel::transition::transition()` | `parcel::transition::tests::happy_path_check_in_out_deliver` |
| Cannot mark Delivered without Check-in | (same) | Guard: `RequiresCheckInExists` | `parcel::machine::tests::delivered_requires_check_in_history` |
| Available transitions per state | `cmd_parcel_available_transitions` | `StateMachine::available_from()` | `parcel::machine::tests::available_from_lists_only_enabled` |
| Parcel history (immutable, hash-chained) | `cmd_parcel_history` | `SqliteTransitionRepo::history()` | `parcel::transition::tests::happy_path_*` (chain continuity asserts) |
| Configurable transition rules | DB table `parcel_transition_rules` | `StateMachine::new(rules, guards)` | `parcel::machine::tests::disabled_rule_rejected` |
| Typed guards (no string typos) | Compile-time `GuardCode` enum | `guard_for()` exhaustive match | `parcel::machine::tests::guard_code_parses_known_and_rejects_unknown` |

## Claims & Dispute Resolution

| Requirement | Backend command | Service function | Test coverage |
|---|---|---|---|
| Submit / withdraw / resolve claims | `cmd_claim_transition` | `claims::machine::apply_transition()` | `claims::machine::tests::draft_to_submitted`, `both_accept_confirms` |
| Two-party confirmation | (same) | `decide_after_response()` | `claims::machine::tests::any_reject_contests`, `both_accept_confirms` |
| 72-hour auto-cancel (background) | Background: `ClaimTimeoutScheduler` | `apply_transition(AutoCancel)` | `claims::timeout::tests::auto_cancel_moves_submitted_to_auto_cancelled` |
| 72-hour auto-cancel (lazy on read) | `cmd_claim_transition` entry path | `enforce_timeout_lazy()` | `claims::timeout::tests::lazy_cancels_when_deadline_has_passed`, `second_lazy_call_*_is_idempotent` |
| One reopen per claim, manager only | `cmd_claim_transition(ManagerReopen)` | `ClaimLifecycleMachine::evaluate()` | `claims::machine::tests::reopen_quota_enforced` |
| Similarity matching | `cmd_find_claim_matches` | `claims::matching::find_matches()` | `claims::matching::tests::identical_claim_scores_one`, `cross_tenant_never_matches` |

## Settlement Workflow

| Requirement | Backend command | Service function | Test coverage |
|---|---|---|---|
| Two-step approval (prepare → approve) | `cmd_settlement_prepare`, `cmd_settlement_approve` | `settlement::approval::prepare/approve_settlement()` | `settlement::approval::tests::happy_two_step_approval` |
| Approver ≠ preparer | (same) | `workflow::next_status()` | `settlement::approval::tests::same_party_cannot_self_approve` |
| Statement generation | `cmd_settlement_statement` | `settlement::statement::generate_statement()` | `settlement::statement::tests::refund_is_deposit_minus_deductions`, `html_escapes_user_text` |
| Balanced double-entry ledger | `cmd_settlement_check_request` | `settlement::payout::post_ledger_for_settlement()` | `settlement::payout::tests::ledger_is_balanced_*` |

## Auth / Session (Operational)

| Requirement | Backend command | Repo impl | Test coverage |
|---|---|---|---|
| User login (argon2 verify + session set) | `cmd_login` | Direct SQL in `commands/auth_cmds.rs` | `db::repos::tests::*` (migration creates users table) |
| Logout (clear session) | `cmd_logout` | `SessionState::clear()` | `ipc::guard::tests::clear_invalidates_session` |
| Current user query | `cmd_current_user` | `SessionState::current()` | (via guard tests) |

## Access Control & IPC Security

| Requirement | Backend command | Mechanism | Test coverage |
|---|---|---|---|
| Role-based permission matrix | All 51 commands | `auth::guard::require()` | `auth::guard::tests::*` (6 tests) |
| Tenant-scoped isolation | All commands | `TenantScope::allows()` | `auth::guard::tests::tenant_scope_is_enforced` |
| IPC permission guard on every handler | All 51 commands | `ipc::guard::require_authenticated()` or `require()` | `ipc::guard::tests::*` (7 tests) |
| Session extract before logic | (same) | `SessionState::current()` | `ipc::guard::tests::unauthenticated_when_no_principal` |
| Structured IPC error envelope | (same) | `IpcError` enum with `type` tag | `ipc::guard::tests::structured_error_serializes_with_type_tag` |

## SQLite Repository Integration

| Trait | SQLite impl | Test file | Tests |
|---|---|---|---|
| `AuditWriter` | `SqliteAuditWriter` | `db/repos/tests.rs` | `audit_writer_inserts_and_enforces_append_only` |
| `ParcelRepository` | `SqliteParcelRepo` | `db/repos/tests.rs` | `parcel_repo_reads_state_and_tenant`, `parcel_repo_updates_state` |
| `TransitionRepository` | `SqliteTransitionRepo` | `db/repos/tests.rs` | `transition_repo_appends_and_retrieves_history` |
| `ClaimRepository` | `SqliteClaimRepo` | `db/repos/tests.rs` | `claim_repo_loads_view_with_deadline`, `claim_repo_applies_status_*` |
| `ExpiredClaimFinder` | `SqliteExpiredClaimFinder` | `db/repos/tests.rs` | `expired_claim_finder_returns_past_deadline` |
| `SettlementRepository` | `SqliteSettlementRepo` | `db/repos/tests.rs` | `settlement_repo_loads_and_updates_status` |
| `ApprovalRepository` | `SqliteApprovalRepo` | `db/repos/tests.rs` | `approval_repo_insert_and_fetch` |
| Migrations | `Database::run_migrations()` | `db/repos/tests.rs` | `migrations_are_idempotent` |
| Failure boundary | — | `db/repos/tests.rs` | `parcel_state_not_updated_if_transition_append_fails` |

## Audit Logging

| Requirement | Backend command | Mechanism | Test coverage |
|---|---|---|---|
| Append-only audit_logs table | Auto: every state-changing command | `SqliteAuditWriter::append()` | `audit::writer` tests + DB triggers |
| System actor attribution | Timeout / scheduler paths | `audit_role_for()` → `AuditRole::System` | Integration tested in `claims::timeout::tests::*` |
| Before/after JSON snapshots | Parcel transition, claim transition | `AuditLog { before_state, after_state }` | Service-level tests capture both states |

## Encryption & Key Management

| Requirement | Backend command | Mechanism | Test coverage |
|---|---|---|---|
| AES-256-GCM field encryption | Internal | `db::encryption::FieldCipher` | `db::encryption::tests::*` (5 tests) |
| OS keystore (not on disk) | Internal | `keys::WindowsCredentialStore` | `keys::tests::*` (7 tests, InMemoryKeyStore) |
| Key rotation (resumable, batched) | Internal | `keys::rotation::run_rotation()` | `keys::rotation::tests::*` (5 tests) |
| Partial masking for identifiers | Internal | `db::masking::mask_national_id()` | `db::masking::tests::*` (5 tests) |

## Document Management

| Requirement | Backend command | Command status | Service tested | Test coverage |
|---|---|---|---|---|
| Chunked upload + resume | `cmd_upload_start/put_chunk/status/finalize/abort` | ✅ SqliteChunkRepo backed | `docs::chunks::*` logic tested | `docs::chunks::tests::*` (8 tests) |
| Atomic rename on finalize | (same) | ✅ SqliteChunkRepo + TmpGuard | TmpGuard RAII tested | `finalize_rolls_back_rename_when_register_fails` |
| Crash recovery for uploads | Startup call | ✅ service ready (no command) | `docs::recovery::cleanup_orphaned_uploads()` | `docs::recovery::tests::*` (7 tests) |
| Offline preview | `cmd_attachment_preview` | ✅ returns metadata from DB | `docs::preview::Previewer` tested | `docs::preview::tests::*` (2 tests) |
| Tag add/remove | `cmd_attachment_add_tag/remove_tag` | ✅ SqliteTagRepo backed | `SqliteTagRepo::add/remove` tested | `db::repos::tests::tag_repo_add_and_remove_are_symmetric` |
| Search | `cmd_attachment_search` | ✅ SqliteAttachmentSearch backed | `SqliteAttachmentSearch::search` tested | `db::repos::tests::attachment_search_returns_empty_for_no_data` |
| Watermarked downloads | `cmd_wrap_with_watermark` | ✅ works live | `sharing::watermark` | `sharing::watermark::tests::*` (6 tests) |

## Scheduling Engine

| Requirement | Backend command | Command status | Test coverage |
|---|---|---|---|
| Constraint validation (hard/soft) | `cmd_schedule_validate` | ✅ works live (pure algorithm) | `scheduling::constraints::tests::*` (7 tests) |
| Greedy slot allocation | `cmd_schedule_propose` | ✅ works live (pure algorithm) | `scheduling::algorithm::tests::*` (9 tests) |
| Versioned rule sets | `cmd_schedule_activate_rule_set` | ✅ SqliteRuleRepo backed | `scheduling::rules::tests::*` (2 tests) + `db::repos::tests::rule_repo_load_active_returns_none_when_empty` |

## Analytics & Experiments

| Requirement | Backend command | Command status | Test coverage |
|---|---|---|---|
| Funnel tracking | `cmd_analytics_funnel` | ✅ works live (pure, empty events) | `analytics::dashboard::tests::funnel_*` (3 tests) |
| Retention cohorts | `cmd_analytics_retention` | ✅ works live (pure, empty input) | `retention_groups_by_cohort_window` |
| Quality metrics | `cmd_analytics_quality` | ✅ works live (pure) | `quality_metrics_basic`, `quality_handles_empty` |
| CSV / JSON export | `cmd_analytics_export` | ✅ works live | `analytics::exports::tests::*` (5 tests) |
| Event tracking | `cmd_analytics_track` | ✅ SqliteEventRepo backed | `analytics::events::tests::*` (5 tests) + `db::repos::tests::event_repo_inserts_and_rolls_up` |
| A/B experiment assignment | `cmd_experiment_assign` | ✅ SqliteExperimentRepo backed | `experiments::tests::*` (5 tests) |

## Update / Recovery

| Requirement | Backend command | Command status | Test coverage |
|---|---|---|---|
| Crash-safe recovery | Startup (no command) | ✅ service ready | `recovery::checkpoint::tests::*` (5 tests) |
| File handle tracking | `cmd_open_handles` | ✅ works live (reads tracker) | `recovery::handles::tests::*` (10 tests) |
| Quiesce before update | — (InstallerOps trait) | Tested with mocks | `update::installer::tests::quiesce_*` (3 tests) |
| Signed update verify | `cmd_update_verify` | ✅ wired with dev Ed25519 public key | `update::verifier::tests::*` (4 tests) |
| Install update | `cmd_update_install` | ✅ ConcreteInstallerOps with HandleQuiescer | `update::installer::tests::*` (7 tests) + `system_cmds::tests::snapshot_copies_db_file` |
| Rollback to N-1 | `cmd_update_rollback` | ✅ ConcreteRollbackOps with HandleQuiescer | `update::rollback::tests::*` (3 tests) + `system_cmds::tests::restore_copies_snapshot_back_to_live` |
| Recovery outcome | `cmd_last_recovery_outcome` | ✅ SqliteRecoveryRepo backed | `db::repos::tests::recovery_repo_records_and_retrieves_outcome` |
| Installed versions | `cmd_list_installed_versions` | ✅ reads from app_versions table | `db::repos::tests::version_repo_inserts_and_queries` |

## Sharing / Data Protection

| Requirement | Backend command | Command status | Test coverage |
|---|---|---|---|
| Password-protected share packages | `cmd_share_build_package` | ✅ works live (requires ExportReport permission) | `sharing::package::tests::*` (6 tests) |
| 7-day expiry enforcement | `cmd_share_verify_access` | ✅ SqlitePackageRepo backed | `sharing::expiry::tests::*` (6 tests) + `db::repos::tests::package_repo_load_returns_none_for_nonexistent` |
| Revoke packages | `cmd_share_revoke` | ✅ SqlitePackageRepo backed | Service tested via mock + SQLite integration |
| Sweep expired | `cmd_share_sweep_expired` | ✅ SqlitePackageRepo backed | `sharing::expiry::tests::sweep_*` |
| Watermark with username + timestamp | `cmd_wrap_with_watermark` | ✅ works live | `sharing::watermark::tests::*` (6 tests) |

## Security Boundary Tests (db/repos/tests.rs)

| Test | Category | Verifies |
|---|---|---|
| `guard_rejects_unauthenticated_on_empty_session` | Unauthenticated | Empty session → IpcError::Unauthenticated |
| `guard_rejects_unauthenticated_after_logout` | Unauthenticated | Clear session → guard fails |
| `liaison_blocked_from_parcel_operate` | Unauthorized | Liaison role denied ParcelOperate |
| `reviewer_blocked_from_approve_settlement` | Unauthorized | Reviewer role denied ApproveSettlement |
| `staff_blocked_from_manage_users` | Unauthorized | Staff role denied ManageUsers |
| `guard_blocks_cross_tenant_access` | Tenant isolation | Tenant A user blocked from tenant B scope |
| `parcel_in_tenant_a_invisible_to_tenant_b_scoped_query` | Object isolation | Parcel tenant mismatch detected |
| `claim_loads_only_for_matching_id` | Object isolation | Non-existent claim returns None, not error |
| `parcel_update_to_same_state_is_idempotent` | Payload edge case | Redundant update succeeds |
| `claim_apply_status_sets_closed_at_on_terminal` | Lifecycle correctness | Terminal status sets closed_at |
| `settlement_status_lifecycle_draft_to_paid` | E2E lifecycle | Full Draft→Pending→Approved→Paid through SQLite |
| `audit_log_delete_blocked_by_trigger` | Append-only enforcement | DELETE on audit_logs blocked |
| `claim_lazy_timeout_autocancel_via_sqlite` | E2E lazy timeout | Submit → deadline pass → auto-cancel via SQLite |
| `expired_finder_ignores_terminal_claims` | Failure boundary | Terminal claims not returned by expired scan |
| `event_repo_inserts_and_rolls_up` | Analytics repo | Event insert + daily aggregate roll-up through SQLite |
| `recovery_repo_records_and_retrieves_outcome` | Recovery repo | Record startup outcome + query last |
| `recovery_repo_last_outcome_returns_most_recent` | Recovery repo | Most recent outcome wins |
| `version_repo_inserts_and_queries` | Version repo | app_versions table CRUD |
| `attachment_search_returns_empty_for_no_data` | Document repo | Empty search returns empty, not error |
| `tag_repo_add_and_remove_are_symmetric` | Document repo | Add/remove tags through SQLite |
| `rule_repo_load_active_returns_none_when_empty` | Scheduling repo | No active rule set returns None |
| `package_repo_load_returns_none_for_nonexistent` | Sharing repo | Missing package returns None |
