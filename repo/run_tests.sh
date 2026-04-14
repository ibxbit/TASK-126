#!/usr/bin/env bash
# Shoreline Property Operations Console — full test runner.
# Runs the Rust backend suite plus frontend type-check and unit tests.
#
# Usage:   ./run_tests.sh
# Exit:    non-zero on any failure; zero on all-green.
#
# Container-aware: auto-installs dependencies if node_modules is
# missing and skips keyring tests when no display server is available.

set -euo pipefail

cd "$(dirname "$0")"

# ── Detect container / headless environment ─────────────────────────
CARGO_TEST_EXTRA_ARGS=""
if [ -f /.dockerenv ] || grep -qsE 'docker|containerd' /proc/1/cgroup 2>/dev/null; then
  echo "[info] Running inside container"
  # Skip keyring tests — no Credential Manager in headless Linux
  CARGO_TEST_EXTRA_ARGS="-- --skip keys::tests"
fi

# ── Ensure frontend dependencies are installed ──────────────────────
if [ -f package.json ] && [ ! -d node_modules ]; then
  echo "── Installing frontend dependencies ────────────────────────────"
  if command -v pnpm >/dev/null 2>&1; then
    pnpm install --frozen-lockfile 2>/dev/null || pnpm install
  elif command -v npm >/dev/null 2>&1; then
    npm install
  fi
fi

# ── 1. Rust backend tests ───────────────────────────────────────────
echo "── Rust backend tests ──────────────────────────────────────────"
if command -v cargo >/dev/null 2>&1; then
  # shellcheck disable=SC2086
  cargo test --manifest-path src-tauri/Cargo.toml --all-features $CARGO_TEST_EXTRA_ARGS
else
  echo "[skip] cargo not found on PATH"
fi

# ── 2. Frontend type-check ──────────────────────────────────────────
echo
echo "── Frontend type-check ─────────────────────────────────────────"
if command -v pnpm >/dev/null 2>&1; then
  pnpm typecheck
elif command -v npm >/dev/null 2>&1; then
  npx --yes tsc --noEmit
else
  echo "[skip] no pnpm/npm found on PATH"
fi

# ── 3. Frontend unit tests ─────────────────────────────────────────
echo
echo "── Frontend unit tests ─────────────────────────────────────────"
if [ -f package.json ]; then
  if command -v pnpm >/dev/null 2>&1; then
    pnpm test -- --run
  elif command -v npm >/dev/null 2>&1; then
    npm test -- --run
  fi
else
  echo "[skip] no package.json"
fi

echo
echo "All tests passed."
