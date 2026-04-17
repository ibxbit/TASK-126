#!/usr/bin/env bash
# Shoreline Property Operations Console — full test runner.
# Runs the Rust backend suite plus frontend type-check and unit tests.
#
# Usage:
#   ./run_tests.sh                # all phases, fail loud on missing tools
#   ./run_tests.sh --coverage     # also emit lcov/html coverage reports
#   ALLOW_HOST_TOOLING=1 ...      # opt out of the "must be Docker if a tool
#                                 # is missing" gate (developers only)
#
# Exit codes:
#   0   all phases ran and passed
#   2   pre-flight failure (a required tool is missing AND we're not in
#       Docker AND ALLOW_HOST_TOOLING is unset). The runner refuses to
#       silently skip a major test phase.
#   non-zero (other)   one or more test phases failed
#
# Container-aware: in Docker we (a) install frontend deps if needed,
# (b) skip OS-keyring tests that require a desktop session, and
# (c) install cargo-llvm-cov on first --coverage run if available.
#
# Phase reporting: every phase prints its outcome, and a final summary
# block is emitted to stdout for the CI parser.

set -euo pipefail

cd "$(dirname "$0")"

# ─── Argument parsing ──────────────────────────────────────────────────
COVERAGE="${COVERAGE:-0}"
for arg in "$@"; do
  case "$arg" in
    --coverage)         COVERAGE=1 ;;
    -h|--help)
      sed -n '2,18p' "$0"
      exit 0
      ;;
    *)
      echo "[error] unknown argument: $arg" >&2
      exit 2
      ;;
  esac
done

# ─── Detect execution context ──────────────────────────────────────────
#
# We distinguish three contexts:
#  1. OUR test container   — has /app + the Rust toolchain + pnpm. Run tests natively.
#  2. CI orchestrator host — likely itself a container (/.dockerenv exists, /proc/1/cgroup
#                            mentions docker), but has NO rust/node installed. It should
#                            delegate to `docker compose run tests`.
#  3. Developer workstation — host OS with tools installed.
#
# The historical check for `/.dockerenv || /proc/1/cgroup` flagged every CI runner as
# "inside container" and then required cargo/pnpm — which meant the CI host (that has
# neither) failed with `[fatal] required tool 'cargo' not found inside container`.
# We now treat missing tools on a "container-but-no-tools" host as a *delegation signal*.
IN_CONTAINER=0
if [ -f /.dockerenv ] || grep -qsE 'docker|containerd' /proc/1/cgroup 2>/dev/null; then
  IN_CONTAINER=1
fi

HAVE_CARGO=0; HAVE_PNPM=0; HAVE_NPM=0; HAVE_DOCKER=0
command -v cargo  >/dev/null 2>&1 && HAVE_CARGO=1
command -v pnpm   >/dev/null 2>&1 && HAVE_PNPM=1
command -v npm    >/dev/null 2>&1 && HAVE_NPM=1
command -v docker >/dev/null 2>&1 && HAVE_DOCKER=1

# Identify OUR test container: presence of /app + cargo. Anything else with
# the in-container signal is treated as an orchestrator host.
IN_OUR_TEST_CONTAINER=0
if [ "$IN_CONTAINER" = "1" ] && [ "$HAVE_CARGO" = "1" ] && [ -d /app/src-tauri ]; then
  IN_OUR_TEST_CONTAINER=1
  echo "[info] Running inside project test container"
fi

# ─── Delegation to docker compose (CI orchestrator path) ──────────────
#
# If we're on a host that can't run the tests natively (missing cargo/pnpm)
# but Docker is available, transparently dispatch to `docker compose run tests`.
# This lets a single `./run_tests.sh` invocation succeed from:
#   • a developer's machine with docker desktop + no local Rust
#   • a CI orchestrator container that has docker but no Rust toolchain
#   • our own test container (runs natively)
TOOLS_MISSING=0
if [ "$HAVE_CARGO" != "1" ] || { [ "$HAVE_PNPM" != "1" ] && [ "$HAVE_NPM" != "1" ]; }; then
  TOOLS_MISSING=1
fi

if [ "$TOOLS_MISSING" = "1" ] && [ "$IN_OUR_TEST_CONTAINER" != "1" ]; then
  if [ "${ALLOW_HOST_TOOLING:-0}" = "1" ]; then
    echo "[warn] required tools missing; ALLOW_HOST_TOOLING=1 — attempting best-effort native run"
  elif [ "$HAVE_DOCKER" = "1" ] && [ -f docker-compose.yml ]; then
    echo "[info] required tools unavailable on host — delegating to 'docker compose run --rm tests'"
    exec docker compose run --rm tests
  else
    cat >&2 <<EOF
[fatal] test tools are not installed and Docker is unavailable.

Pick one of:

  • Install Rust (cargo) + pnpm/npm on this host, OR
  • Install Docker and run: docker compose run tests
  • Acknowledge a partial run by exporting ALLOW_HOST_TOOLING=1

EOF
    exit 2
  fi
fi

CARGO_TEST_EXTRA_ARGS=""
if [ "$IN_OUR_TEST_CONTAINER" = "1" ]; then
  # No Windows Credential Manager in headless Linux — keys::tests would
  # error. Skipping is intentional and documented in README.
  CARGO_TEST_EXTRA_ARGS="-- --skip keys::tests"
fi

# ─── Phase status accumulators ────────────────────────────────────────
RUST_RESULT="skipped"
TYPE_RESULT="skipped"
VITEST_RESULT="skipped"
OVERALL_FAIL=0

# ─── Ensure frontend dependencies are installed ──────────────────────
if [ -f package.json ] && [ ! -d node_modules ]; then
  echo "── Installing frontend dependencies ──────────────────────────"
  if [ "$HAVE_PNPM" = "1" ]; then
    pnpm install --frozen-lockfile 2>/dev/null || pnpm install
  elif [ "$HAVE_NPM" = "1" ]; then
    npm install
  fi
fi

# ─── 1. Rust backend tests ────────────────────────────────────────────
echo
echo "── Rust backend tests ────────────────────────────────────────"
if [ "$HAVE_CARGO" = "1" ]; then
  if [ "$COVERAGE" = "1" ]; then
    if command -v cargo-llvm-cov >/dev/null 2>&1; then
      mkdir -p target/coverage
      if cargo llvm-cov --manifest-path src-tauri/Cargo.toml \
        --all-features --lcov --output-path target/coverage/rust-lcov.info \
        $CARGO_TEST_EXTRA_ARGS; then
        RUST_RESULT="pass"
      else
        RUST_RESULT="fail"; OVERALL_FAIL=1
      fi
    else
      echo "[warn] --coverage requested but cargo-llvm-cov is not installed; falling back to cargo test"
      echo "[warn] install with: cargo install cargo-llvm-cov"
      # shellcheck disable=SC2086
      if cargo test --manifest-path src-tauri/Cargo.toml --all-features $CARGO_TEST_EXTRA_ARGS; then
        RUST_RESULT="pass"
      else
        RUST_RESULT="fail"; OVERALL_FAIL=1
      fi
    fi
  else
    # shellcheck disable=SC2086
    if cargo test --manifest-path src-tauri/Cargo.toml --all-features $CARGO_TEST_EXTRA_ARGS; then
      RUST_RESULT="pass"
    else
      RUST_RESULT="fail"; OVERALL_FAIL=1
    fi
  fi
fi

# ─── 2. Frontend type-check ───────────────────────────────────────────
echo
echo "── Frontend type-check ───────────────────────────────────────"
if [ "$HAVE_PNPM" = "1" ]; then
  if pnpm typecheck; then TYPE_RESULT="pass"; else TYPE_RESULT="fail"; OVERALL_FAIL=1; fi
elif [ "$HAVE_NPM" = "1" ]; then
  if npx --yes tsc --noEmit; then TYPE_RESULT="pass"; else TYPE_RESULT="fail"; OVERALL_FAIL=1; fi
fi

# ─── 3. Frontend unit + journey tests ─────────────────────────────────
echo
echo "── Frontend unit + journey tests ─────────────────────────────"
VITEST_ARGS="--run"
if [ "$COVERAGE" = "1" ]; then VITEST_ARGS="--coverage --run"; fi
if [ "$HAVE_PNPM" = "1" ]; then
  # shellcheck disable=SC2086
  if pnpm test -- $VITEST_ARGS; then VITEST_RESULT="pass"; else VITEST_RESULT="fail"; OVERALL_FAIL=1; fi
elif [ "$HAVE_NPM" = "1" ]; then
  # shellcheck disable=SC2086
  if npm test -- $VITEST_ARGS; then VITEST_RESULT="pass"; else VITEST_RESULT="fail"; OVERALL_FAIL=1; fi
fi

# ─── Phase summary (machine-readable) ────────────────────────────────
echo
echo "── Phase summary ─────────────────────────────────────────────"
printf "  rust:     %s\n" "$RUST_RESULT"
printf "  typecheck: %s\n" "$TYPE_RESULT"
printf "  vitest:   %s\n" "$VITEST_RESULT"
echo
if [ "$OVERALL_FAIL" -ne 0 ]; then
  echo "[fail] one or more test phases failed."
  exit 1
fi
# Refuse to report success if a major phase was silently skipped.
if [ "$RUST_RESULT" = "skipped" ] || [ "$TYPE_RESULT" = "skipped" ] || [ "$VITEST_RESULT" = "skipped" ]; then
  echo "[fail] a required test phase was skipped — refusing to report success."
  exit 2
fi
echo "All tests passed."
