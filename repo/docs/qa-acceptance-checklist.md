# QA Acceptance Checklist — Shoreline Property Operations Console

Version: 0.1.0  
Date: 2026-04-14  
Tester: ____________________  

> Mark each item: **P** (Pass), **F** (Fail), **N/A**, or **B** (Blocked).  
> A full "Pass" requires zero **F** items in Sections 1–8.

---

## 1. Automated Test Suite (Gate — must pass before manual QA)

| # | Check | How to verify | Status |
|---|-------|---------------|--------|
| 1.1 | Rust backend tests pass (229 tests) | `cargo test --manifest-path src-tauri/Cargo.toml --all-features` | |
| 1.2 | TypeScript type-check passes | `pnpm typecheck` | |
| 1.3 | Frontend unit tests pass (Vitest) | `pnpm test --run` | |
| 1.4 | All-in-one runner succeeds | `.\run_tests.ps1` (Windows) or `./run_tests.sh` (Unix) | |
| 1.5 | DPI verification script passes | `.\scripts\verify_dpi.ps1` | |
| 1.6 | Installer verification passes | `.\scripts\verify_installer.ps1` | |

---

## 2. UI / UX — Login Flow

| # | Check | Expected | Status |
|---|-------|----------|--------|
| 2.1 | App launches to login screen | LoginForm visible with "Sign in to continue" | |
| 2.2 | Username field is auto-focused | Cursor in username field on load | |
| 2.3 | Both fields are required | HTML validation prevents empty submit | |
| 2.4 | Password field obscures input | `type="password"` — dots shown | |
| 2.5 | Valid credentials → dashboard | Enter valid user/pass → workspace cards appear | |
| 2.6 | Invalid credentials → error | Enter wrong password → red error message, no crash | |
| 2.7 | Submit button disables during auth | Shows "Signing in..." and is disabled while loading | |
| 2.8 | Sign-out returns to login | Click "Sign out" → login form reappears | |

---

## 3. UI / UX — Dashboard & Workspaces

| # | Check | Expected | Status |
|---|-------|----------|--------|
| 3.1 | Dashboard shows 3 workspace cards | Move-Out Case, Parcel Queue, Claims Inbox | |
| 3.2 | Username + role displayed in header | e.g. "admin (Administrator)" | |
| 3.3 | Click workspace card opens new window | New Tauri window opens with correct title | |
| 3.4 | Multiple instances of same workspace | Click same card twice → two separate windows | |
| 3.5 | Workspace window shows correct title | Title bar matches workspace name | |
| 3.6 | Footer shows version info | "Offline-first · v0.1.0 · Tauri + React + Rust + SQLite" | |

---

## 4. UI / UX — Window Management

| # | Check | Expected | Status |
|---|-------|----------|--------|
| 4.1 | Main window: 1600×1000 default | Window opens at 1600×1000 logical pixels | |
| 4.2 | Main window: 1280×720 minimum | Cannot resize below 1280×720 | |
| 4.3 | Workspace windows: correct sizes | Move-Out 1280×860, Parcel 1100×720, Claims 1280×820 | |
| 4.4 | All windows resizable | Drag edges to resize — no lockup | |
| 4.5 | Windows centered on open | New windows appear centered on screen | |
| 4.6 | Close workspace window | Close button works, main dashboard unaffected | |

---

## 5. DPI / High-DPI Scaling

| # | Check | Expected | Status |
|---|-------|----------|--------|
| 5.1 | App renders correctly at 100% scale | Normal sizing, crisp text | |
| 5.2 | App renders correctly at 125% scale | All text readable, no clipping, no overlap | |
| 5.3 | App renders correctly at 150% scale | UI elements scale proportionally | |
| 5.4 | App renders correctly at 200% scale | No layout breakage, buttons clickable | |
| 5.5 | Login form centered at all scales | Form card centered in viewport | |
| 5.6 | Dashboard cards wrap at narrow width | Grid reflows when window is shrunk | |
| 5.7 | Font rendering is crisp | Segoe UI renders at native DPI, not blurry | |
| 5.8 | System tray icon not blurry | Tray icon renders at appropriate resolution | |
| 5.9 | Context menus native rendering | Right-click menus use OS-native rendering | |

**How to test DPI:**  
Settings → Display → Scale and layout → Change to 125%/150%/200%.  
Relaunch app after each change.  
Alternatively: per-app DPI override via Properties → Compatibility → High DPI settings.

---

## 6. System Tray, Shortcuts & Reminders

| # | Check | Expected | Status |
|---|-------|----------|--------|
| 6.1 | Tray icon appears on launch | Icon visible in system tray area | |
| 6.2 | Tray tooltip shows app name | Hover → "Shoreline Property Ops" | |
| 6.3 | Ctrl+K triggers quick search | Global shortcut fires shortcut event | |
| 6.4 | Ctrl+Shift+N triggers new case | Global shortcut fires shortcut event | |
| 6.5 | F2 triggers rename | Global shortcut fires shortcut event | |
| 6.6 | Reminder scheduling works | Schedule a reminder → verify timer starts | |
| 6.7 | Reminder fires notification | Wait for scheduled time → notification appears | |
| 6.8 | Cancel reminder works | Cancel a pending reminder → it does not fire | |

---

## 7. Installer / MSI Flow

| # | Check | Expected | Status |
|---|-------|----------|--------|
| 7.1 | MSI builds without error | `pnpm tauri build` exits 0 | |
| 7.2 | MSI file size reasonable | 3–100 MB (depends on WebView2 bundling) | |
| 7.3 | MSI installs silently | `msiexec /i <path> /qn` exits 0 | |
| 7.4 | Install creates program directory | `%ProgramFiles%\Shoreline Property Operations Console` | |
| 7.5 | Executable runs after install | Double-click .exe → app launches | |
| 7.6 | Start Menu shortcut created | Shoreline entry in Start Menu | |
| 7.7 | Add/Remove Programs entry | Listed in Settings → Apps | |
| 7.8 | Uninstall removes program directory | `msiexec /x <path> /qn` → directory removed | |
| 7.9 | AppData preserved after uninstall | User data in `%APPDATA%\Shoreline` not deleted | |
| 7.10 | Reinstall over existing works | Install same version again → no error | |
| 7.11 | Upgrade preserves data | Install new version → user data intact | |

---

## 8. Offline Operation & Data Integrity

| # | Check | Expected | Status |
|---|-------|----------|--------|
| 8.1 | App launches with no network | Disconnect Wi-Fi → launch → app works | |
| 8.2 | No outbound network requests | Check CSP: `default-src 'self'` blocks external | |
| 8.3 | SQLite DB created on first run | `%APPDATA%\Shoreline\shoreline.db` exists | |
| 8.4 | DB uses WAL mode | `PRAGMA journal_mode` returns `wal` | |
| 8.5 | Foreign keys enforced | `PRAGMA foreign_keys` returns 1 | |
| 8.6 | Audit log is append-only | DELETE on `audit_logs` blocked by trigger | |
| 8.7 | OS keystore integration | Credentials stored in Windows Credential Manager | |
| 8.8 | Encrypted fields at rest | Sensitive fields encrypted with AES-256-GCM | |

---

## 9. Security & Access Control

| # | Check | Expected | Status |
|---|-------|----------|--------|
| 9.1 | Unauthenticated IPC blocked | Call any command without login → Unauthenticated error | |
| 9.2 | Role-based permission enforced | Liaison cannot ParcelOperate, Reviewer cannot ApproveSettlement | |
| 9.3 | Tenant isolation enforced | Tenant A cannot access Tenant B parcels/claims | |
| 9.4 | Session clears on logout | After logout, session state is empty | |
| 9.5 | Argon2 password verification | Login uses argon2 hashing — no plaintext comparison | |
| 9.6 | CSP blocks script injection | `script-src 'self'` — no inline/eval | |

---

## 10. Update & Recovery (Functional Verification)

| # | Check | Expected | Status |
|---|-------|----------|--------|
| 10.1 | Verify command accepts valid package | `cmd_update_verify` returns manifest JSON | |
| 10.2 | Install creates snapshot | Snapshot directory created in backups/ | |
| 10.3 | Snapshot includes DB + WAL files | DB file and WAL (if present) copied to snapshot | |
| 10.4 | Rollback restores from snapshot | Rollback copies snapshot files back to live location | |
| 10.5 | File handles quiesced before snapshot | HandleQuiescer called before any file copy | |
| 10.6 | Version gate blocks mismatched updates | Package with wrong min_required_version rejected | |
| 10.7 | Duplicate version rejected | Attempting to install same version twice fails | |
| 10.8 | Recovery checkpoint on startup | Recovery event recorded at app startup | |

---

## 11. Edge Cases & Error Handling

| # | Check | Expected | Status |
|---|-------|----------|--------|
| 11.1 | Rapid double-click workspace card | Two windows open, no crash | |
| 11.2 | Close all workspace windows | Dashboard remains functional | |
| 11.3 | Login with empty fields | HTML validation prevents submission | |
| 11.4 | Network cable pull during operation | App continues — no hang, no crash | |
| 11.5 | Force-close and relaunch | App recovers — no DB corruption | |
| 11.6 | Same-state parcel transition | Idempotent — no error | |
| 11.7 | 72-hour claim auto-cancel | Expired claim auto-cancelled on next read | |
| 11.8 | Settlement self-approval blocked | Same user cannot both prepare and approve | |

---

## Sign-Off

| Role | Name | Date | Result |
|------|------|------|--------|
| QA Tester | | | Pass / Fail |
| Dev Lead | | | Pass / Fail |
| Project Manager | | | Accepted / Rejected |

### Criteria for Acceptance
- **Full Pass**: All items in Sections 1–8 marked **P** or **N/A**, zero **F**.
- **Conditional Pass**: Sections 9–11 may have **F** items with documented mitigation plan.
- **Fail**: Any **F** in Sections 1–8.
