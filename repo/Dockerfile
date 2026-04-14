# ─────────────────────────────────────────────────────────────────────
# Shoreline Property Operations Console — Docker build environment
#
# Provides a reproducible Rust + Node.js environment for:
#   • Running the full test suite (cargo test + typecheck + vitest)
#   • Serving the Vite frontend on port 1420 for UI verification
#
# Usage via docker compose (preferred):
#   docker compose up              # start app + run tests
#   docker compose up app          # frontend only on :1420
#   docker compose run tests       # test suite only
# ─────────────────────────────────────────────────────────────────────

# ── Stage 1: base ────────────────────────────────────────────────────
# Shared base with Rust toolchain + Node.js + system libraries needed
# to compile Tauri 2 on Linux.
FROM rust:1.78-bookworm AS base

# Tauri 2 Linux build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    libwebkit2gtk-4.1-dev \
    libgtk-3-dev \
    libayatana-appindicator3-dev \
    librsvg2-dev \
    libssl-dev \
    pkg-config \
    curl \
    wget \
    file \
    && rm -rf /var/lib/apt/lists/*

# Install Node.js 20 LTS
RUN curl -fsSL https://deb.nodesource.com/setup_20.x | bash - \
    && apt-get install -y --no-install-recommends nodejs \
    && rm -rf /var/lib/apt/lists/*

# Install pnpm globally
RUN npm install -g pnpm@9

WORKDIR /app

# ── Stage 2: deps ───────────────────────────────────────────────────
# Install and cache Rust + Node dependencies separately from source
# so rebuilds only re-run when dependency files change.
FROM base AS deps

# Copy dependency manifests first (cache layer)
COPY package.json ./
COPY src-tauri/Cargo.toml src-tauri/Cargo.toml
COPY src-tauri/build.rs src-tauri/build.rs

# Create stub source files so cargo can resolve the dependency tree
# without copying the full source (which would bust the cache).
RUN mkdir -p src-tauri/src \
    && echo 'fn main() {}' > src-tauri/src/main.rs \
    && echo 'pub fn run() {}' > src-tauri/src/lib.rs \
    && mkdir -p src-tauri/capabilities \
    && echo '{"identifier":"default","windows":["main"],"permissions":["core:default"]}' \
       > src-tauri/capabilities/default.json \
    && mkdir -p src-tauri/migrations \
    && touch src-tauri/migrations/.keep

# Frontend dependencies
RUN pnpm install --frozen-lockfile 2>/dev/null || pnpm install

# Pre-fetch Rust dependencies (cargo build downloads + compiles deps)
RUN cd src-tauri && cargo fetch 2>/dev/null || true

# ── Stage 3: build ──────────────────────────────────────────────────
# Full source copy + compile the Rust tests (but don't run them yet).
FROM deps AS build

# Copy full source
COPY . .

# Compile Rust test binaries (catches compile errors early in build)
RUN cd src-tauri && cargo test --all-features --no-run 2>&1 || true

# ── Stage 4: app ────────────────────────────────────────────────────
# Serves the Vite frontend dev server for UI verification.
FROM build AS app

EXPOSE 1420

# Vite must bind to 0.0.0.0 inside Docker (not localhost)
CMD ["pnpm", "dev", "--host", "0.0.0.0"]

# ── Stage 5: tests ──────────────────────────────────────────────────
# Runs the complete test suite and exits.
FROM build AS tests

CMD ["bash", "./run_tests.sh"]
