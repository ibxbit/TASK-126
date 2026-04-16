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

# ─── Detect container vs host ──────────────────────────────────────────
IN_CONTAINER=0
if [ -f /.dockerenv ] || grep -qsE 'docker|containerd' /proc/1/cgroup 2>/dev/null; then
  IN_CONTAINER=1
  echo "[info] Running inside container"
fi

CARGO_TEST_EXTRA_ARGS=""
if [ "$IN_CONTAINER" = "1" ]; then
  # No Windows Credential Manager in headless Linux — keys::tests would
  # error. Skipping is intentional and documented in README.
  CARGO_TEST_EXTRA_ARGS="-- --skip keys::tests"
fi

# ─── Pre-flight: required tools must be present ───────────────────────
#
# Outside Docker we *refuse* to silently skip a phase: a missing tool
# means the user must either install it, set ALLOW_HOST_TOOLING=1 to
# acknowledge the partial run, or use the Docker path
# (`docker compose run tests`).
require_tool() {
  local tool="$1"
  if command -v "$tool" >/dev/null 2>&1; then return 0; fi
  if [ "$IN_CONTAINER" = "1" ]; then
    # Inside Docker, a missing tool is a build-image bug. Fail loud.
    echo "[fatal] required tool '$tool' not found inside container" >&2
    exit 2
  fi
  if [ "${ALLOW_HOST_TOOLING:-0}" = "1" ]; then
    echo "[warn] '$tool' missing on host; ALLOW_HOST_TOOLING=1 — skipping its phase"
    return 1
  fi
  cat >&2 <<EOF
[fatal] required tool '$tool' is not installed.

This runner does not silently skip test phases. Either:

  • Install '$tool' on the host, OR
  • Use the Docker path:   docker compose run tests
  • Acknowledge a partial run by exporting ALLOW_HOST_TOOLING=1

EOF
  exit 2
}

HAVE_CARGO=0
HAVE_PNPM=0
HAVE_NPM=0

if require_tool cargo; then HAVE_CARGO=1; fi
if command -v pnpm >/dev/null 2>&1; then
  HAVE_PNPM=1
elif command -v npm >/dev/null 2>&1; then
  # pnpm is preferred but not strictly required if npm is available.
  HAVE_NPM=1
else
  if [ "$IN_CONTAINER" = "1" ]; then
    echo "[fatal] neither pnpm nor npm available inside container" >&2
    exit 2
  fi
  if [ "${ALLOW_HOST_TOOLING:-0}" != "1" ]; then
    echo "[fatal] neither pnpm nor npm found on host. Install pnpm 9 or set ALLOW_HOST_TOOLING=1." >&2
    exit 2
  fi
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
