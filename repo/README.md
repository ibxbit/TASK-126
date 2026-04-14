# Shoreline Property Operations Console

Offline-first desktop application for property operations — move-out cases, parcel queues, and claim resolution. Tauri shell + React UI + Rust domain logic + SQLite persistence. Runs entirely on the local machine with no network services.

---

## Architecture at a glance

```
┌─────────────────────────────────────────────────────────────────┐
│  Presentation  (React + TypeScript, Vite, Tauri WebView2)       │
│     src/                                                        │
└──────────────────────────┬──────────────────────────────────────┘
                           │  Tauri IPC (typed commands + events)
                           ▼
┌─────────────────────────────────────────────────────────────────┐
│  Application  (Rust)                                            │
│     src-tauri/src/                                              │
│       auth/       ipc/        parcel/     claims/               │
│       settlement/ docs/       scheduling/ analytics/            │
│       sharing/    keys/       update/     recovery/             │
│       audit/      tray/       windows/    shortcuts/   menu/    │
└──────────────────────────┬──────────────────────────────────────┘
                           ▼
┌─────────────────────────────────────────────────────────────────┐
│  Persistence  (Local only)                                      │
│    • SQLite  — %APPDATA%/Shoreline/shoreline.db (WAL)           │
│    • Files   — %APPDATA%/Shoreline/attachments/<tenant>/…       │
│    • Keys    — Windows Credential Manager                       │
└─────────────────────────────────────────────────────────────────┘
```

Every domain module is self-contained and trait-driven so repositories can be swapped between in-memory test doubles and concrete SQLite implementations. Cross-cutting concerns — auth guards, audit writes, handle tracking — are composed via dedicated modules (`ipc::guard`, `audit::writer`, `recovery::handles`).

---

## Prerequisites

| Tool | Version | Install |
|---|---|---|
| Rust | 1.75+ | `rustup install stable` |
| Node.js | 18+ | https://nodejs.org |
| pnpm | 9+ | `npm install --global pnpm` |
| Tauri CLI | 2.x | Installed transitively by `pnpm install` |
| WebView2 Runtime | — | Ships with Windows 11 |

On Windows the MSI build additionally requires **WiX Toolset v3** (auto-fetched by `pnpm tauri build` on first use) and, for signed release builds, an Authenticode code-signing certificate.

---

## Initial setup

```powershell
# from the repo root
pnpm install

# one-time: generate bundle icons from a 1024×1024 PNG source
pnpm tauri icon path\to\source.png
```

See `src-tauri/icons/README.md` for icon details. `pnpm tauri dev` will start without icons; `pnpm tauri build` requires them.

---

## One-click start (Docker)

The fastest way to start the project and verify all services:

```bash
docker compose up
```

This single command builds and launches two services:

| Service | Port | Purpose | Health check |
|---------|------|---------|--------------|
| **app** | `1420` | Vite/React frontend dev server | `curl http://localhost:1420` |
| **tests** | — | Full test suite (Rust + TS + Vitest) | Exit code 0 = pass |

**Verify services are reachable:**

```bash
# Frontend UI — should return HTML
curl -s http://localhost:1420 | head -5

# Test results — check exit code
docker compose logs tests
docker compose ps tests    # State: "Exited (0)" = all pass
```

**Run only the tests:**

```bash
docker compose run tests
```

**Run only the app (frontend):**

```bash
docker compose up app
# Then open http://localhost:1420 in a browser
```

**Rebuild after code changes:**

```bash
docker compose up --build
```

> **Note:** The full desktop application (with native windows, tray icon, and
> WebView2) requires a Windows host — see the native development section below.
> The Docker setup provides the frontend UI server and the complete test suite
> in a reproducible container environment.

---

## Native development (Windows)

For the full desktop experience with Tauri windows, system tray, and shortcuts:

```powershell
pnpm tauri dev
```

Vite serves the frontend at `http://localhost:1420` and Tauri opens the WebView2 window. First build compiles Cargo dependencies (several minutes); subsequent starts hit the app window in **under 5 seconds** on a typical office PC.

**Release (signed `.msi`):**

```powershell
pnpm tauri build
```

Output lands in `src-tauri\target\release\bundle\msi\`. For code-signing, set `bundle.windows.signCommand` in `src-tauri/tauri.conf.json` to your Authenticode signing command before building.

---

## Services and ports

| Service / Component | Port | Protocol | Exposed in Docker | Notes |
|---------------------|------|----------|-------------------|-------|
| Vite dev server | 1420 | HTTP | Yes (`app` service) | Frontend React UI; `strictPort: true` |
| Tauri IPC | — | Internal | No | Rust ↔ WebView2 bridge (desktop only) |
| SQLite database | — | File | No | `%APPDATA%/Shoreline/shoreline.db` (WAL mode) |
| Windows Credential Manager | — | OS API | No | Encryption key storage (desktop only) |

The application is **offline-first** by design — it opens no outbound network connections and exposes no API endpoints beyond the Vite dev server used during development.

---

## Running the test suite

### Everything (Docker — recommended)

```bash
docker compose run tests
```

### Everything (native)

```powershell
.\run_tests.ps1            # Windows
./run_tests.sh             # Unix (WSL / CI)
```

The runner executes the Rust suite first (fails fast on compile / test errors), then the frontend type-check, then the vitest unit tests. A green run prints `All tests passed.` and exits 0.

The script is container-aware: when running inside Docker it auto-installs dependencies and skips keyring tests (which require a display server).

### Rust only

```bash
cargo test --manifest-path src-tauri/Cargo.toml --all-features
```

Covers every `#[cfg(test)] mod tests` in the workspace — auth guards, state machines (parcel + claim + settlement), matching algorithm, scheduling engine, analytics dashboards + experiments, document chunking + finalize safety, watermarking, share packages, key rotation, recovery, handle quiescer, update installer + rollback, audit integration, IPC permission guards.

### Frontend only

```powershell
pnpm typecheck
pnpm test
```

---

## Verification checklist

### Docker verification (delivery acceptance)

```bash
# 1. One-click start
docker compose up -d

# 2. Verify app service is reachable on port 1420
curl -f http://localhost:1420          # should return HTML with <div id="root">

# 3. Verify tests pass
docker compose run tests              # exit code 0 = all green
docker compose logs tests             # review output

# 4. Check service health
docker compose ps                     # app: "Up (healthy)", tests: "Exited (0)"

# 5. Teardown
docker compose down
```

### Native desktop verification

Once `pnpm tauri dev` has launched the app:

1. **Main window opens** — titled "Shoreline Property Operations Console", 1600×1000, resizable down to 1280×720.
2. **Dashboard renders** — three workspace cards (Move-Out Case, Parcel Queue, Claims Inbox).
3. **Multi-window works** — clicking any card spawns a new window with label `<workspace>:<uuid>`; multiple clicks produce multiple parallel instances (see `windows::cmd_open_workspace`).
4. **Tray icon present** — Windows system tray shows "Shoreline Property Ops" with Open / Quick Search / Quit menu; left-click re-focuses main window.
5. **Global shortcuts registered** — listen in DevTools Console via:
   ```js
   const { listen } = await import("@tauri-apps/api/event");
   await listen("shortcut://fired", e => console.log(e.payload));
   ```
   then press Ctrl+K, Ctrl+Shift+N, F2 — each fires an event with the matching `action`.
6. **Reminders fire locally** — schedule one for a few seconds ahead:
   ```js
   const { invoke } = await import("@tauri-apps/api/core");
   await invoke("cmd_schedule_reminder", {
     reminder: { id: crypto.randomUUID(), title: "test",
                 fire_at_unix: Math.floor(Date.now()/1000) + 5 }
   });
   ```
   A `reminder://fired` event arrives within a second of the deadline.
7. **Offline confirmed** — the process opens no network sockets. Verify in PowerShell:
   ```powershell
   Get-NetTCPConnection -OwningProcess (Get-Process shoreline).Id
   ```
   Returns no listening or outbound connections (ignore IPv6 loopback from WebView2's dev-server during `tauri dev`).
8. **Local data directories exist** once first DB access occurs:
   ```powershell
   Get-ChildItem $env:APPDATA\Shoreline
   ```
   shows `shoreline.db` (WAL mode), `attachments/`, and a `lockfile` while the app is running. The lockfile disappears on graceful shutdown and is detected on the next startup to trigger recovery.
9. **Encryption keys live in the OS keystore** — `Control Panel → User Accounts → Credential Manager → Windows Credentials` contains `ShorelinePropertyOps / master_key.v1` after first launch. Keys are never written to disk.

---

## Project layout

```
repo/
├── README.md
├── Dockerfile                          # multi-stage build (Rust + Node)
├── docker-compose.yml                  # one-click start: app + tests
├── .dockerignore
├── run_tests.sh / run_tests.ps1        # test runners (host & container)
├── package.json / pnpm-lock.yaml       # frontend
├── index.html                          # Vite entry
├── vite.config.ts / tsconfig.json
├── scripts/                            # QA verification scripts
│   ├── verify_dpi.ps1
│   └── verify_installer.ps1
├── docs/                               # QA & coverage documentation
│   ├── coverage-mapping.md
│   └── qa-acceptance-checklist.md
├── src/
│   ├── main.tsx / App.tsx              # React shell
│   ├── ipc/                            # typed IPC wrappers (one file per domain)
│   ├── hooks/                          # useShortcuts, useParcelMachine, …
│   ├── components/                     # ContextMenu, …
│   └── smoke.test.ts
└── src-tauri/
    ├── Cargo.toml
    ├── tauri.conf.json
    ├── build.rs
    ├── capabilities/default.json
    ├── icons/                          # populated by `pnpm tauri icon`
    ├── migrations/                     # 0001 … 0011 SQL DDL
    └── src/
        ├── main.rs                     # binary entry → shoreline::run()
        ├── lib.rs                      # module root + Tauri Builder
        ├── analytics/  audit/  auth/  claims/  db/  docs/
        ├── ipc/  keys/  menu/  parcel/  recovery/
        ├── scheduling/  settlement/  sharing/  shortcuts/
        ├── tray/  update/  windows/
        └── …
```

Each module carries its own unit tests (`#[cfg(test)] mod tests`). Run them in isolation with:

```bash
cargo test --manifest-path src-tauri/Cargo.toml auth::
cargo test --manifest-path src-tauri/Cargo.toml parcel::machine
```

---

## What's wired vs. pending

| Subsystem | Status |
|---|---|
| React shell + dashboard + workspace windows | ✅ wired, clickable |
| All 51 IPC commands registered + permission-guarded | ✅ every TS `invoke()` has a matching `#[tauri::command]`; zero stubs remaining |
| Login UI (React form + argon2 password verify) | ✅ operational — login gate before dashboard |
| Auth login/logout/current_user | ✅ `cmd_login` / `cmd_logout` / `cmd_current_user` |
| System tray + global shortcuts + reminder ticker | ✅ wired |
| Auth / roles / permission matrix + IPC guard | ✅ 8 guard tests + 14 security-boundary integration tests |
| State machines (parcel / claim / settlement) | ✅ service logic complete + unit tested |
| SQLite database (WAL, 11 migrations, connection manager) | ✅ wired at startup; idempotency tested |
| SQLite repos: all domains | ✅ 12 concrete repo impls across parcel, claims, settlement, audit, documents, analytics, scheduling, sharing, recovery, versions |
| Settlement statement hydration from DB | ✅ queries deposits + deduction_items + residents |
| Claim lazy timeout via SQLite | ✅ end-to-end tested |
| Document upload/finalize/abort (chunked) | ✅ SqliteChunkRepo backed |
| Document tag add/remove + search | ✅ SqliteTagRepo + SqliteAttachmentSearch backed |
| Document preview | ✅ returns attachment metadata from DB; full file-read needs FieldCipher in Tauri state |
| Encryption + key management + rotation | ✅ tested with in-memory keystore |
| Update verify (Ed25519) | ✅ wired with dev public key (replace at release) |
| Update install + rollback | ✅ ConcreteInstallerOps / ConcreteRollbackOps with HandleQuiescer |
| Recovery outcome query | ✅ SqliteRecoveryRepo backed |
| Installed versions list | ✅ reads from app_versions table |
| Scheduling validate + propose | ✅ loads rule set from DB (falls back to empty) |
| Scheduling activate_rule_set | ✅ SqliteRuleRepo backed |
| Analytics track | ✅ SqliteEventRepo — inserts into events + daily_event_aggregates |
| Analytics funnel / retention / quality | ✅ queries events table by tenant + time range |
| Analytics export (CSV / JSONL) | ✅ pure function, works live |
| A/B experiment assign | ✅ SqliteExperimentRepo backed |
| Share build package | ✅ AES-256 encrypted ZIP, permission-gated |
| Share verify access / revoke / sweep | ✅ SqlitePackageRepo backed |
| Watermarked downloads | ✅ works live |

See `docs/coverage-mapping.md` for the full requirement → command → test matrix.

---

## Troubleshooting

- **`docker compose up` is slow on first run** — the initial build downloads the Rust toolchain, Node.js, and system libraries (~5–10 min). Subsequent builds use Docker layer caching and complete in under 30 seconds unless `Cargo.toml` or `package.json` change.
- **Port 1420 conflict with Docker** — stop any native `pnpm tauri dev` process first, or remap: `docker compose run -p 3000:1420 app`.
- **`docker compose run tests` fails with keyring error** — this should be auto-skipped inside the container. If it persists, run with `-- --skip keys::tests` appended.
- **`pnpm tauri dev` fails with "icon not found"** — run `pnpm tauri icon <source.png>` once, or temporarily remove the `bundle.icon` entries from `tauri.conf.json` for a dev-only sanity check.
- **`cargo test` fails with a `keyring` error in a headless environment** — Credential Manager is unavailable on CI runners. Filter with `cargo test -- --skip keys::tests` to skip those integration-style checks; they run fine on a real Windows desktop session.
- **WebView2 errors on Windows** — install or update the [Evergreen WebView2 Runtime](https://developer.microsoft.com/en-us/microsoft-edge/webview2/).
- **Port 1420 already in use** — `vite.config.ts` sets `strictPort: true` intentionally; free the port or edit both the Vite config and `tauri.conf.json > build.devUrl` in lockstep.
