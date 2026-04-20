# Forge dev-workflow runner.
#
# Install just: `cargo install just` (or `brew install just`, `apt install just`).
# Run `just` with no args to see all recipes.

set shell := ["bash", "-ceuo", "pipefail"]

# Show available recipes.
default:
    @just --list

# -----------------------------------------------------------------------------
# Dev workflow
# -----------------------------------------------------------------------------

# Run the desktop app in dev mode. Spawns Vite at :5173 via Tauri's
# `beforeDevCommand`, then launches the shell webview against it.
dev:
    @command -v cargo-tauri >/dev/null || { echo >&2 "cargo-tauri not found. Install: cargo install tauri-cli --version '^2.0' --locked"; exit 1; }
    cd crates/forge-shell && cargo tauri dev

# Start only the Vite dev server (use with `just dev-shell` in another terminal).
dev-vite:
    cd web && pnpm --filter app dev

# Launch only the Tauri shell (Vite must already be running on :5173).
dev-shell:
    cargo run -p forge-shell

# Build everything: Rust workspace (debug) + full pnpm workspace.
build:
    cargo build --workspace
    cd web && pnpm install --frozen-lockfile && pnpm -r build

# Release build of the three shippable binaries. The Tauri shell still
# loads from web/packages/app/dist, so the pnpm build is required.
release-bins:
    cd web && pnpm install --frozen-lockfile && pnpm -r build
    cargo build --release -p forge-cli -p forge-session -p forge-shell

# Auto-format Rust sources.
fmt:
    cargo fmt --all

# -----------------------------------------------------------------------------
# CI-mirrored checks — CI calls these recipes directly
# -----------------------------------------------------------------------------

# Rust lane: fmt --check, cargo check, clippy (warnings denied), rustdoc.
# Mirrors the Rust lint steps in .github/workflows/ci.yml `check` job.
check-rust:
    cargo fmt --all -- --check
    cargo check --all-targets
    cargo clippy --workspace --all-targets -- -D warnings
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features

# Web lane: typecheck + design-token drift gate.
# Mirrors the pnpm lint steps in the `frontend` job. Assumes deps installed.
check-web:
    cd web && pnpm -r typecheck
    cd web && pnpm check-tokens

# Both lanes.
check: check-rust check-web

# Rust test suite.
test-rust:
    cargo test --all

# Web test suite.
test-web:
    cd web && pnpm -r test

# Both lanes.
test: test-rust test-web

# Verify generated TS bindings are in sync with Rust types. ts-rs emits the
# TS files as a side effect of `cargo build` (see forge-core/src/ids.rs `ts`
# attribute), so run `just build` first.
ts-check:
    git diff --exit-code web/packages/ipc/src/generated/

# Supply-chain audits. Local use only; CI uses dedicated actions for caching
# and for surfacing advisories as PR annotations. cargo-deny consults the
# same RustSec advisory DB as cargo-audit while also enforcing licenses,
# bans, and sources — see docs/dev/security.md.
# Requires: cargo install cargo-deny
audit:
    cargo deny check --all-features
    cd web && pnpm audit --audit-level moderate

# -----------------------------------------------------------------------------
# Phase 1 smoke
# -----------------------------------------------------------------------------

# Phase 1 smoke gate — build + CLI-only UATs (UAT-09, UAT-10, UAT-13).
# Fastest pre-Phase-2 confidence check; no browser required.
smoke:
    cargo build --workspace
    ./docs/testing/phase1-uat.sh --cli-only

# -----------------------------------------------------------------------------
# Cleanup
# -----------------------------------------------------------------------------

# Drop all build artifacts (Rust + web).
clean:
    cargo clean
    rm -rf web/packages/app/dist web/packages/*/node_modules web/node_modules
