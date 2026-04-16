# Shoreline Property Operations Console

**Project type: desktop**

Offline-first desktop application for property operations — move-out cases, parcel queues, and claim resolution. Tauri shell + React UI + Rust domain logic + SQLite persistence. Runs entirely on the local machine with no network services.

---

## Quick start (Docker — primary workflow)

Docker is the primary, reproducible way to build and verify this project. No local Rust, Node.js, or native toolchain installation is required.

```bash
docker compose up
```

This single command builds and launches two services:

| Service | Port | Purpose | Health check |
|---------|------|---------|--------------|
| **app** | `1420` | Vite/React frontend dev server | `curl http://localhost:1420` |
| **tests** | — | Full test suite (Rust + TS + Vitest) | Exit code 0 = pass |

### Startup verification

```bash
# 1. Start all services
docker compose up -d

# 2. Verify frontend is reachable (should return HTML with <div id="root">)
curl -f http://localhost:1420

# 3. Run the full test suite — exit code 0 means all tests pass
docker compose run tests

# 4. Check service health
docker compose ps                     # app: "Up (healthy)", tests: "Exited (0)"

# 5. View test results
docker compose logs tests

# 6. Teardown
docker compose down
```

### Access method

Open `http://localhost:1420` in a browser to access the frontend UI (served by the `app` Docker service).

### Run only the tests

```bash
docker compose run tests
```

### Rebuild after code changes

```bash
docker compose up --build
```

> **Note:** The full desktop application (with native windows, tray icon, and
> WebView2) requires a Windows host — see the optional native development
> section below. The Docker setup provides the frontend UI server and the
> complete test suite in a reproducible container environment.

---

## Authentication

The application uses local authentication with argon2id password hashing. Users and credentials are stored in the local SQLite database. There is no external identity provider.

### Demo credentials

The following demo accounts are available for testing. In the **Docker test environment** (`docker compose run tests`), these credentials are used by the automated test suite against the in-memory fake backend. In the **native desktop app** (`pnpm tauri dev`), the database starts empty and users must be seeded via the SQLite database before first login.

| Username | Password | Role | Scope | Notes |
|----------|----------|------|-------|-------|
| `admin` | `admin123` | Administrator | Global (`*`) | Full configuration and user management |
| `pm_alice` | `alice123` | Property Manager | Tenant-scoped | Approves settlements, reopens claims |
| `staff_bob` | `bob123` | Staff | Tenant-scoped | Parcel operations, resident submissions |
| `reviewer_carol` | `carol123` | Reviewer | Tenant-scoped | Read-only access with export and audit |
| `liaison_dan` | `dan123` | Liaison | Tenant-scoped | Resident data entry only |

**Role permission summary:**

| Role | Key permissions |
|------|----------------|
| Administrator | All permissions: configure rules/templates/permissions, manage users, approve settlements, reopen claims, export, audit |
| Property Manager | Approve settlements, reopen claims, parcel operations, export, audit |
| Staff | Parcel operations, accept resident submissions, view claims |
| Reviewer | View claims, view resident data, export reports, audit log read |
| Liaison | Input/view resident data only |

---

## Native development (optional — Windows only)

For the full desktop experience with Tauri windows, system tray, and shortcuts, install the native toolchain:

| Tool | Version | Install |
|---|---|---|
| Rust | 1.75+ | `rustup install stable` |
| Node.js | 18+ | https://nodejs.org |
| pnpm | 9+ | `npm install --global pnpm` |
| Tauri CLI | 2.x | Installed transitively by `pnpm install` |
| WebView2 Runtime | — | Ships with Windows 11 |

```powershell
pnpm install
pnpm tauri dev
```

Vite serves the frontend at `http://localhost:1420` and Tauri opens the WebView2 window.

---

## Services and ports

| Service / Component | Port | Protocol | Exposed in Docker | Notes |
|---------------------|------|----------|-------------------|-------|
| Vite dev server | 1420 | HTTP | Yes (`app` service) | Frontend React UI; `strictPort: true` |
| Tauri IPC | — | Internal | No | Rust <-> WebView2 bridge (desktop only) |
| SQLite database | — | File | No | `%APPDATA%/Shoreline/shoreline.db` (WAL mode) |
| Windows Credential Manager | — | OS API | No | Encryption key storage (desktop only) |

The application is **offline-first** by design — it opens no outbound network connections and exposes no API endpoints beyond the Vite dev server used during development.

---

## Running the test suite

### Everything (Docker — recommended)

```bash
docker compose run tests
```

### Everything (native — optional)

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

### Frontend only

```powershell
pnpm typecheck
pnpm test
```

### Frontend coverage

```bash
pnpm test:coverage
```

Current measured coverage (production code, excluding test infrastructure):

| Metric | Coverage |
|--------|----------|
| Statements | 99.61% |
| Branches | 97.09% |
| Functions | 98.85% |
| Lines | 99.61% |

---

## Architecture at a glance

```
+-----------------------------------------------------------------+
|  Presentation  (React + TypeScript, Vite, Tauri WebView2)       |
|     src/                                                        |
+-----------------------------+-----------------------------------+
                              |  Tauri IPC (typed commands + events)
                              v
+-----------------------------------------------------------------+
|  Application  (Rust)                                            |
|     src-tauri/src/                                              |
|       auth/       ipc/        parcel/     claims/               |
|       settlement/ docs/       scheduling/ analytics/            |
|       sharing/    keys/       update/     recovery/             |
|       audit/      tray/       windows/    shortcuts/   menu/    |
+-----------------------------+-----------------------------------+
                              v
+-----------------------------------------------------------------+
|  Persistence  (Local only)                                      |
|    - SQLite  -- %APPDATA%/Shoreline/shoreline.db (WAL)          |
|    - Files   -- %APPDATA%/Shoreline/attachments/<tenant>/...    |
|    - Keys    -- Windows Credential Manager                      |
+-----------------------------------------------------------------+
```

Every domain module is self-contained and trait-driven so repositories can be swapped between in-memory test doubles and concrete SQLite implementations. Cross-cutting concerns — auth guards, audit writes, handle tracking — are composed via dedicated modules (`ipc::guard`, `audit::writer`, `recovery::handles`).

---

## Project layout

```
repo/
+-- README.md
+-- Dockerfile                          # multi-stage build (Rust + Node)
+-- docker-compose.yml                  # one-click start: app + tests
+-- .dockerignore
+-- run_tests.sh / run_tests.ps1        # test runners (host & container)
+-- package.json / pnpm-lock.yaml       # frontend
+-- index.html                          # Vite entry
+-- vite.config.ts / tsconfig.json
+-- docs/                               # QA & coverage documentation
+-- src/
|   +-- main.tsx / App.tsx              # React shell
|   +-- ipc/                            # typed IPC wrappers (one file per domain)
|   +-- hooks/                          # useShortcuts, useParcelMachine, ...
|   +-- components/                     # ContextMenu, LoginForm, Dashboard, ...
|   +-- journeys/                       # cross-boundary journey tests
|   +-- test/                           # fake backend + test utilities
|   +-- smoke.test.ts
+-- src-tauri/
    +-- Cargo.toml
    +-- tauri.conf.json
    +-- build.rs
    +-- capabilities/default.json
    +-- icons/                          # populated by `pnpm tauri icon`
    +-- migrations/                     # 0001 ... 0011 SQL DDL
    +-- src/
        +-- main.rs                     # binary entry -> shoreline::run()
        +-- lib.rs                      # module root + Tauri Builder
        +-- analytics/  audit/  auth/  claims/  db/  docs/
        +-- ipc/  keys/  menu/  parcel/  recovery/
        +-- scheduling/  settlement/  sharing/  shortcuts/
        +-- tray/  update/  windows/
        +-- commands/                   # Tauri command handlers + lifecycle tests
```

---

## Troubleshooting

- **`docker compose up` is slow on first run** — the initial build downloads the Rust toolchain, Node.js, and system libraries (~5-10 min). Subsequent builds use Docker layer caching and complete in under 30 seconds unless `Cargo.toml` or `package.json` change.
- **Port 1420 conflict with Docker** — stop any native `pnpm tauri dev` process first, or remap: `docker compose run -p 3000:1420 app`.
- **`docker compose run tests` fails with keyring error** — this should be auto-skipped inside the container. If it persists, run with `-- --skip keys::tests` appended.
- **`pnpm tauri dev` fails with "icon not found"** — run `pnpm tauri icon <source.png>` once, or temporarily remove the `bundle.icon` entries from `tauri.conf.json` for a dev-only sanity check.
- **`cargo test` fails with a `keyring` error in a headless environment** — Credential Manager is unavailable on CI runners. Filter with `cargo test -- --skip keys::tests`; they run fine on a real Windows desktop session.
- **Port 1420 already in use** — `vite.config.ts` sets `strictPort: true` intentionally; free the port or edit both the Vite config and `tauri.conf.json > build.devUrl` in lockstep.
