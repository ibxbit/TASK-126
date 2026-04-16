# Shoreline Property Ops — Comprehensive Test Plan

Target: **>90% meaningful coverage** across the whole stack (Rust backend, IPC
boundary, React frontend, and offline-desktop UX).  "Meaningful" here means we
test _what the user relies on_ — real lifecycles, real persistence, real
permission enforcement — not just "every line is touched by some assertion".

The plan is organized as five concentric rings:

```
  E2E (headed Tauri smoke)        ← real binary, real SQLite, real WebView
  UI-in-browser (Playwright)      ← Vite served, Tauri IPC mocked at the edge
  IPC contract integration        ← Rust command fn invoked w/ real state
  Module integration              ← service + real SQLite repo
  Unit / pure logic               ← state machines, pure helpers, formatters
```

Every requirement in [`coverage-mapping.md`](coverage-mapping.md) is mapped to
at least one ring, and the higher-value flows (auth, state transitions,
settlement approval, upload finalize, sharing, tray reminders, global
shortcuts) are covered at **two or more** rings so the product is protected
even if a single mock drifts from reality.

---

## 1. Architecture under test

```
 React/TSX  ─(src/ipc/*.ts)─►  Tauri IPC  ─(cmd_*)─►  Rust services
                                                          │
                                                          ▼
                                                    SQLite (WAL)
                                                          │
                                                          ▼
                                                   FS (attachments/)
                                                          │
                                                          ▼
                                                  OS keystore (keys)
```

Boundaries where bugs cluster:

1. **Session state** — a handler forgets to call `guard::require_*` and
   leaks tenant-scoped data.
2. **Type / shape drift** between Rust structs (`serde`) and TS interfaces.
3. **Transactional coupling** — mutation persists but audit row doesn’t, or
   vice-versa.
4. **Async UI** — loading / error states diverge from IPC reality.
5. **Offline-first promises** — the app calls a network when it shouldn't,
   or mishandles a crash/resume.

The plan below aims at all five.

---

## 2. Coverage targets by ring

| Ring | Framework | Target coverage | Evidence |
|---|---|---|---|
| Unit / pure logic (Rust) | `cargo test` | ≥95% line, ≥95% branch | `cargo llvm-cov` report in `target/coverage/` |
| Module integration (Rust+SQLite) | `cargo test` (in-memory SQLite) | ≥90% line on `db::repos::*`, all state machines | same |
| IPC contract (Rust) | `cargo test` with `tauri::test::mock_app` | every `cmd_*` exercised, ≥90% of happy + error branches | same |
| Unit / component (TS) | `vitest + jsdom + @testing-library` | ≥90% line on `src/**` | c8 reporter (`coverage/lcov.info`) |
| UI-in-browser / smoke | `@playwright/test` (Chromium, WebKit) | every workspace route + login + logout | `playwright-report/` |
| Desktop E2E | `tauri-driver` + `webdriverio` (Windows only, nightly) | launch → login → open workspace → close | `e2e/reports/` |

Docker runs ring 1–5. Ring 6 runs on a real Windows host (out-of-band in CI,
per the README’s offline-first requirement).

---

## 3. Critical user journeys & cross-cutting scenarios

The matrix below drives the **new tests** added in this PR. Each row is
anchored to the requirement in `coverage-mapping.md` and points to the test
file that enforces it.

### 3.1 Authentication & session

| Journey | Rings exercised | Test file |
|---|---|---|
| New user logs in with correct password → session set → dashboard rendered | IPC, UI, smoke | `src-tauri/src/commands/auth_cmds.rs::tests`, `src/components/LoginForm.test.tsx`, `src/App.test.tsx`, `e2e/login.spec.ts` |
| Login fails: unknown user | IPC, UI | same + error-path case |
| Login fails: wrong password | IPC, UI | same |
| Login fails: account disabled | IPC | `auth_cmds.rs::tests::login_rejects_inactive_user` |
| Login fails: user has no role assigned | IPC | `auth_cmds.rs::tests::login_rejects_user_with_no_role` |
| Logout clears the session | IPC, UI | `auth_cmds.rs::tests::logout_clears_session`, `src/ipc/auth.test.ts` |
| `currentUser()` returns the live principal, including global vs tenant scope | IPC | `auth_cmds.rs::tests::current_user_reports_global_and_tenant_scope` |
| Child workspace window inherits session (no re-login prompt) | UI, smoke | `src/App.test.tsx` workspace-path branches, `e2e/login.spec.ts` |

### 3.2 Parcel lifecycle

| Journey | Rings | Test file |
|---|---|---|
| Check-in → Check-out → Delivered happy path; history is hash-chained | Module, IPC | `parcel::transition::tests::happy_path_*` (existing), + `parcel_cmds.rs::tests::transition_creates_audit_row` (new) |
| Cannot mark Delivered without a prior Check-in | Module | `parcel::machine::tests::delivered_requires_check_in_history` (existing) |
| A disabled rule is rejected even if the state shape permits it | Module | `parcel::machine::tests::disabled_rule_rejected` (existing) |
| Guard code round-trips through DB as a typed enum; unknown codes are rejected at load | Module | `parcel::machine::tests::guard_code_parses_known_and_rejects_unknown` (existing) |
| `cmd_parcel_available_transitions` honors current state | IPC | `parcel_cmds.rs::tests::available_transitions_*` (extended) |
| Liaison is denied `ParcelOperate` through the IPC guard | IPC, security | `auth::guard::tests::liaison_blocked_from_parcel_operate` (existing) + frontend negative test |

### 3.3 Claims

| Journey | Rings |
|---|---|
| Submit → UnderReview → both_accept → Confirmed | Module |
| Auto-cancel after 72 h (both lazy-on-read and background scheduler) | Module |
| Reopen quota: only one reopen allowed; manager-only | Module |
| Cross-tenant claim never matches in similarity search | Module |

All of the above are already covered by existing tests; this plan ADDS the IPC
contract layer (`commands/claim_cmds.rs::tests`) so the wire shape is verified.

### 3.4 Settlement

| Journey | Rings |
|---|---|
| Two-step approval: preparer ≠ approver | Module, IPC |
| Statement HTML escapes user-supplied strings | Module |
| Ledger posts are balanced (Σ debits = Σ credits) | Module |
| Approver without `ApproveSettlement` permission is denied at IPC | IPC, security |

### 3.5 Documents

| Journey | Rings |
|---|---|
| Chunked upload: start → 3 `put_chunk`s → `finalize` → version row + SHA256 match | Module, IPC |
| Resume after mid-flight crash — `upload_status` reports missing chunks | Module |
| `abort` deletes staging, does NOT register a version | Module, IPC |
| Tags add/remove are idempotent | Module |
| Watermarked preview requires an authenticated session | IPC, security |
| Cross-tenant search returns only the caller’s tenant | Module |

### 3.6 Sharing

| Journey | Rings |
|---|---|
| Build AES-256 encrypted ZIP package; password-required | Module |
| Package expires after 7 days; `verify_access` rejects expired | Module, IPC |
| Revoke immediately invalidates a package | Module, IPC |
| `sweep_expired` removes only past-deadline packages | Module |
| `ExportReport` permission enforced on `cmd_share_build_package` | IPC, security |

### 3.7 Scheduling

| Journey | Rings |
|---|---|
| Hard/soft constraint validation | Module |
| Greedy slot allocator honors capacity | Module |
| Active rule set resolves to the latest; empty DB returns None | Module |

### 3.8 Analytics

| Journey | Rings |
|---|---|
| Event track → daily aggregate rollup | Module |
| Funnel, retention, and quality queries return deterministic results on known inputs | Module |
| CSV / JSONL export round-trips | Module, UI (download helper) |
| A/B experiment assignment is sticky per `subject_id` | Module |

### 3.9 System, recovery, update

| Journey | Rings |
|---|---|
| Recovery outcome recorded on startup; latest wins | Module |
| Open-handle tracker lists only open handles | Module |
| Ed25519 signed update verification; tampered bytes rejected | Module |
| Install → success → recorded in `app_versions` | Module |
| Rollback restores N-1 snapshot | Module |

### 3.10 Desktop UX (windows, tray, shortcuts)

| Journey | Rings |
|---|---|
| `ReminderScheduler::schedule` adds to heap; `cancel` removes by id; `pending_count` reports the heap size | Unit (new) |
| Min-heap fires the nearest deadline first | Unit (new) |
| `WindowRegistry::register/unregister/snapshot/count_of` correctly multiplex parallel workspace instances | Unit (new) |
| `Workspace::as_str/route/title/default_size` map consistently (no typos drifting between URL and title) | Unit (new) |
| Context-menu spec serde round-trips action/separator/submenu | Unit (new) |
| Global shortcut action `as_str` stays stable (frontend switches on it) | Unit (new) |

### 3.11 Permission matrix exhaustiveness

Every (role, permission) pair in `roles::Role::permissions()` has a positive
test AND the complement (an un-granted permission is denied). New test file:
`src-tauri/src/auth/roles_matrix.rs::tests` — one `#[test]` per role.

### 3.12 Offline-first guarantee

We assert programmatically that the Rust crate never opens a network socket
during tests (which doubles as a regression alarm if a dep ever pulls in
reqwest). See `src-tauri/src/offline_guard.rs::tests` (new) and the Docker
test that scans the compiled test binary for `tcp_` / `http` symbols as a
sanity check.

---

## 4. Test harness changes

### 4.1 Vitest: jsdom + jest-dom setup, coverage

* `vite.config.ts` now declares `setupFiles: ["./src/test-setup.ts"]` so
  `@testing-library/jest-dom` matchers are registered once globally and
  test files can drop the redundant import.
* Coverage: v8 provider, lcov + html + text reporters → `coverage/`.

### 4.2 Frontend IPC mocking — two strategies

**Strategy A — per-call mocks (unit tests)**. Individual IPC test files
mock `@tauri-apps/api/core` at the Vitest layer (`vi.mock`) so each
function is tested in isolation. See `src/ipc/auth.test.ts`.

**Strategy B — fake backend (integration / journey tests)**. A full
in-memory IPC dispatcher (`src/test/fake-backend.ts`) replaces `invoke`
once, then every command in a journey hits a single coherent backend with
real state management across calls. This catches drifted command names,
argument shapes, auth gates, and cross-command state bugs. Journey tests
in `src/journeys/` use this strategy exclusively, and integration tests
for hooks and components (`*.integration.test.{ts,tsx}`) also use it.

### 4.3 Fake backend domain coverage

The fake backend now implements handlers for **all IPC domains**:

| Domain | Commands | State management |
|---|---|---|
| Auth | login, logout, current_user | Session tracking |
| Desktop | open/close/list windows, reminders, shortcuts, context menu | Window registry, reminder heap |
| Parcel | available_transitions, transition_parcel, parcel_history | State machine with chain hashing |
| Claims | claim_transition, find_claim_matches | Status machine, tenant-scoped matching |
| Settlement | transition, prepare, approve, statement, check_request | Two-step approval, balanced ledger |
| Documents | upload start/put/status/finalize/abort, search, tag, preview | Chunked sessions, attachment store |
| Sharing | watermark, build/verify/revoke/sweep packages | Password protection, expiry |
| Scheduling | activate_rule_set, validate, propose | Overlap detection, greedy assignment |
| Analytics | track, funnel, retention, quality, export, experiment_assign | Event store, sticky A/B |
| System | recovery, handles, verify/install/rollback updates, versions | Version tracking, rollback |

### 4.4 Playwright E2E

Playwright tests live in `e2e/` and run against the Vite dev server with
an injected `__TAURI_INTERNALS__` mock that routes `invoke()` to an
in-page fake. This tests the actual built React app in a real browser
(Chromium) with real DOM, CSS, and navigation.

Config: `e2e/playwright.config.ts`. Run: `pnpm test:e2e`.

### 4.5 Rust: `tauri::test`

Rust IPC tests use `tauri::test::mock_app()` where we need an `AppHandle`
(context-menu / windows), and the plain function call for commands that only
need `tauri::State`. We avoid over-mocking by always constructing a real
in-memory `Database` with all migrations applied — the same code path the
production app takes on startup.

### 4.6 CI matrix

```
jobs:
  rust:        cargo test + cargo llvm-cov → upload lcov
  frontend:    pnpm test --coverage --run  → upload lcov
  playwright:  pnpm test:e2e              → headless Chromium; upload report
  e2e-windows: tauri-driver + webdriverio  → nightly on Windows runner
```

Docker reproduces the first three (keyring tests are skipped inside the
container, per the existing `run_tests.sh` detection).

---

## 5. Untestable areas & mitigations

These cannot be fully automated; we document the compensating controls.

| Area | Why untestable in CI | Mitigation |
|---|---|---|
| Windows Credential Manager reads/writes | Requires a real Windows session | `keys::tests` run on a Windows developer box and on a nightly Windows CI runner; `InMemoryKeyStore` covers business logic. |
| WebView2 → Rust command round-trip with real IPC | No WebView2 on Linux Docker | Covered by Playwright (UI + mocked IPC) and `tauri-driver` E2E on Windows. |
| Signed MSI install | Needs Authenticode + WiX + signed cert | Manual pre-release step in `docs/qa-acceptance-checklist.md`; `update::verifier` covers the signature math with a dev key. |
| Tray icon / right-click menu | Requires a real desktop compositor | Manual step in the acceptance checklist; the **data** that drives it (reminders heap, menu spec serde) is exercised in unit tests. |
| Global shortcuts firing | OS-level hook | Same. `ShortcutAction::as_str` stability is unit-tested so the payload can’t drift. |
| File system errors (full disk, permission denied) | Non-deterministic | Inject via `std::io::Error` in a custom failing `ChunkRepository` double; covered by `docs::chunks::tests::finalize_rolls_back_rename_when_register_fails`. |

---

## 6. New test inventory (coverage improvement)

### 6.1 Cross-boundary journey tests (`src/journeys/`)

These tests drive **real IPC wrappers** through the **fake backend** with
no per-call mocks. Each test verifies command names, argument shapes,
state transitions, error envelopes, and auth gates end-to-end.

| File | Tests | Scenarios covered |
|---|---|---|
| `auth-and-workspace.journey.test.tsx` | 15 | Login/logout session, permission boundaries, workspace open/close, reminder events, full UI journey through React App |
| `parcel-lifecycle.journey.test.ts` | 8 | Full lifecycle (checked_in→receipt_confirmed), chain hashing, terminal states, invalid transitions, unauthenticated rejection |
| `claims.journey.test.ts` | 11 | Draft→confirmed happy path, withdraw, manager reject, auto-cancel, matching (tenant-scoped), event bus |
| `settlement.journey.test.ts` | 12 | Two-step approval (preparer≠approver), workflow transitions, statement with refund calc, balanced ledger, check request, printArtifact |
| `docs-upload.journey.test.ts` | 13 | Chunked upload lifecycle, finalize rejection on missing chunks, abort, search with text/tag filters, idempotent tagging, preview |
| `sharing.journey.test.ts` | 7 | Watermark, package build→verify→revoke cycle, expiry sweep, defaultExpiryUnix, downloadPackage helper |
| `scheduling.journey.test.ts` | 7 | Rule-set activation, overlap detection, clean validation, proposal assignment, unfulfilled demands |
| `analytics.journey.test.ts` | 12 | Event tracking (4 categories), funnel/retention/quality dashboards, CSV/JSONL export, sticky A/B experiment, downloadAs helper |
| `system.journey.test.ts` | 15 | Recovery outcome, open handles, update verify (.spkg validation), install with version tracking, rollback to N-1, version listing |

**Total journey tests: 100**

### 6.2 Component integration tests (fake backend, no mocks)

| File | Tests | Scenarios covered |
|---|---|---|
| `src/components/Dashboard.test.tsx` | 8 | Title/subtitle/footer rendering, user display, all 3 workspace cards with descriptions, click→openWorkspace through dispatcher, multi-window, sign out, role-specific display |
| `src/components/WorkspaceView.test.tsx` | 7 | All 3 workspace routes render correct title, no login gate, subpath handling, domain-ready subtitle, unauthenticated workspace rendering |
| `src/components/LoginForm.integration.test.tsx` | 5 | Login via real IPC, wrong password error from backend, disabled account, unknown user, button re-enable after failure |
| `src/components/ContextMenu.integration.test.tsx` | 2 | Context menu IPC through dispatcher, unauthenticated propagation |

**Total component integration tests: 22**

### 6.3 Hook integration tests (fake backend, no mocks)

| File | Tests | Scenarios covered |
|---|---|---|
| `src/hooks/useParcelMachine.integration.test.ts` | 5 | Load transitions from dispatcher, apply transition with state update, invalid transition error, refresh after state change, unauthenticated mount error |
| `src/hooks/useShortcuts.integration.test.ts` | 4 | Event dispatch from fake bus, unregistered action handling, unmount cleanup, rapid-fire events |

**Total hook integration tests: 9**

### 6.4 Playwright E2E tests (`e2e/`)

| File | Tests | Scenarios covered |
|---|---|---|
| `e2e/auth.e2e.ts` | 6 | Login form visibility, successful login→dashboard, failed login error+re-enable, unknown user, sign out→login, loading state |
| `e2e/dashboard.e2e.ts` | 6 | All workspace cards with descriptions, footer, user identity, card clickability, all cards trigger backend |
| `e2e/workspace.e2e.ts` | 6 | All 3 workspace routes render, no auth gate on workspace, ready message, viewport styling |

**Total Playwright E2E tests: 18**

### 6.5 Summary: test count improvement

| Category | Before | After | Delta |
|---|---|---|---|
| Rust backend tests | 254 | 254 | — |
| Frontend unit tests (mocked) | 73 | 73 | — |
| Journey tests (fake backend) | 15 | 100 | **+85** |
| Component integration tests | 0 | 22 | **+22** |
| Hook integration tests | 0 | 9 | **+9** |
| Playwright E2E tests | 0 | 18 | **+18** |
| **Total** | **342** | **476** | **+134** |

### 6.6 Coverage gaps addressed

| Gap (from audit) | How addressed |
|---|---|
| No true E2E tests | Playwright tests in `e2e/` run against real Vite app in Chromium |
| No real FE↔BE integration | Journey tests for all 10 IPC domains through fake backend |
| Frontend UI modules lack tests | Dashboard, WorkspaceView, LoginForm integration tests added |
| Heavy mocking | Integration tests use coherent fake backend, not per-call mocks |
| Missing edge case assertions | Every journey covers success + failure + auth guard paths |

---

## 7. Rollout

1. PR-1 — adds test plan, fills **unit + IPC contract** gaps on
   both sides, and wires coverage reporters.
2. PR-2 (this change) — adds **cross-boundary journey tests** for all
   domains, **component/hook integration tests**, **Playwright E2E**,
   and extends the fake backend to cover the full IPC surface.
3. PR-3 — `tauri-driver` E2E on Windows nightly.

Each PR must leave the test suite green in Docker (`docker compose run
tests`) and show a coverage delta that moves the module toward ≥90%.
