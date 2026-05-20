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

# One-shot bootstrap for everything `just dev` needs:
#   - verifies rustc / cargo and node >= 20 are on PATH
#   - enables corepack and pins pnpm to the version in web/package.json
#   - installs cargo-tauri 2.x (the `cargo tauri dev` driver)
#   - runs `pnpm install` in web/ so the SolidJS workspace is ready
#   - on Linux, checks the Tauri webkit2gtk system libs via pkg-config and
#     prints the right apt / dnf command if any are missing (sudo step left
#     to the user; this recipe never escalates)
#
# Idempotent — safe to re-run after pulling main.
# Bootstrap a dev environment so `just dev` runs (tauri-cli, pnpm deps, prereq checks).
dev-setup:
    @echo "==> Checking Rust toolchain"
    @command -v rustc >/dev/null || { echo >&2 "rustc not found. Install rustup: https://rustup.rs"; exit 1; }
    @command -v cargo >/dev/null || { echo >&2 "cargo not found. Install rustup: https://rustup.rs"; exit 1; }
    @rustc --version
    @echo "==> Checking Node >= 20"
    @command -v node >/dev/null || { echo >&2 "node not found. Install Node.js >= 20 (https://nodejs.org or via your package manager)."; exit 1; }
    @node_major=$(node -p 'process.versions.node.split(".")[0]'); \
      if [ "$node_major" -lt 20 ]; then echo >&2 "Node $(node -v) is too old; need >= 20."; exit 1; fi
    @echo "    $(node -v)"
    @echo "==> Enabling pnpm via corepack"
    @command -v corepack >/dev/null || { echo >&2 "corepack not found. It ships with Node >= 16.10; reinstall Node or run \`npm i -g corepack\`."; exit 1; }
    corepack enable
    corepack prepare pnpm@9.12.0 --activate
    @pnpm --version
    @echo "==> Installing cargo-tauri (if missing)"
    @command -v cargo-tauri >/dev/null || cargo install tauri-cli --version '^2.0' --locked
    @echo "==> Checking Tauri webkit2gtk system libs (Linux only)"
    @if [ "$(uname -s)" = "Linux" ]; then \
      command -v pkg-config >/dev/null || { echo >&2 "pkg-config missing — see the per-distro hint below."; }; \
      missing=""; \
      for pc in webkit2gtk-4.1 javascriptcoregtk-4.1 libsoup-3.0 gtk+-3.0 librsvg-2.0; do \
        if ! pkg-config --exists "$pc" 2>/dev/null; then missing="$missing $pc"; fi; \
      done; \
      if [ -n "$missing" ]; then \
        echo "    missing pkg-config modules:$missing"; \
        if command -v dnf >/dev/null; then \
          echo "    sudo dnf install -y webkit2gtk4.1-devel libsoup3-devel gtk3-devel librsvg2-devel pkgconf-pkg-config gcc"; \
        elif command -v apt-get >/dev/null; then \
          echo "    sudo apt-get install -y libwebkit2gtk-4.1-dev libsoup-3.0-dev libjavascriptcoregtk-4.1-dev libgtk-3-dev librsvg2-dev pkg-config build-essential"; \
        else \
          echo "    install equivalents of: webkit2gtk-4.1, javascriptcoregtk-4.1, libsoup-3.0, gtk+-3.0, librsvg-2.0, pkg-config, a C toolchain"; \
        fi; \
        echo "    rerun \`just dev-setup\` once installed."; \
      else \
        echo "    all present"; \
      fi; \
    else \
      echo "    skipped (non-Linux host — Tauri uses the OS webview)"; \
    fi
    @echo "==> Installing web workspace deps"
    cd web && pnpm install --frozen-lockfile
    @echo "==> Done. Run \`just dev\` to launch the app."

# Run the desktop app in dev mode. Spawns Vite at :5173 via Tauri's
# `beforeDevCommand`, then launches the shell webview against it.
#
# `WEBKIT_DISABLE_DMABUF_RENDERER=1` works around WebKitGTK failing to
# realize a GL context on hosts that don't expose a DRM render node
# (e.g. systems where `/dev/dri/` has only `card*` and no `renderD*`).
# Without this flag the shell window opens and immediately aborts with
# "GDK failed to realize the GL context". Setting it forces the
# software-composited renderer, which works everywhere at a small perf
# cost — safe to leave on for dev.
dev:
    @command -v cargo-tauri >/dev/null || { echo >&2 "cargo-tauri not found. Install: cargo install tauri-cli --version '^2.0' --locked"; exit 1; }
    cd crates/forge-shell && WEBKIT_DISABLE_DMABUF_RENDERER=1 cargo tauri dev

# Start only the Vite dev server (use with `just dev-shell` in another terminal).
dev-vite:
    cd web && pnpm --filter app dev

# Launch only the Tauri shell (Vite must already be running on :5173).
# Same `WEBKIT_DISABLE_DMABUF_RENDERER` rationale as `just dev`.
dev-shell:
    WEBKIT_DISABLE_DMABUF_RENDERER=1 cargo run -p forge-shell

# Build everything: Rust workspace (debug) + full pnpm workspace.
build:
    cargo build --workspace
    cd web && pnpm install --frozen-lockfile && pnpm -r build

# Release build of the three shippable binaries. The Tauri shell still
# loads from web/packages/app/dist, so the pnpm build is required.
release-bins:
    cd web && pnpm install --frozen-lockfile && pnpm -r build
    cargo build --release -p forge-cli -p forge-session -p forge-shell

# Drives `cargo tauri build`, which runs `beforeBuildCommand` (production
# pnpm build) and then bundles `forge-shell`. Pass a comma-separated bundle
# list to narrow the targets — e.g. `just bundle rpm`, `just bundle deb`,
# `just bundle rpm,deb`. Defaults to `all`, which honours `tauri.conf.json`
# (.deb / .rpm / .AppImage on Linux; .dmg on macOS; .msi on Windows).
# Output lands under `target/release/bundle/<format>/`.
# Production bundle: release-mode Tauri installers for the host platform.
bundle bundles="all":
    @command -v cargo-tauri >/dev/null || { echo >&2 "cargo-tauri not found. Install: cargo install tauri-cli --version '^2.0' --locked"; exit 1; }
    cd web && pnpm install --frozen-lockfile
    cd crates/forge-shell && cargo tauri build --bundles {{bundles}}

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

# Web lane: typecheck + design-token drift gate + raw-button gate.
# Mirrors the pnpm lint steps in the `frontend` job. Assumes deps installed.
# `check-voice` is informational and lives in its own recipe; promote
# it into this lane once the corpus is clean (F-699 follow-up #820).
check-web:
    cd web && pnpm -r typecheck
    cd web && pnpm check-tokens
    cd web && pnpm check-raw-buttons

# Voice-terminology helper (F-699 follow-up #820, item 6). Informational:
# always exits 0, prints findings for human triage. Promote into
# `check-web` once the corpus is clean.
check-voice:
    cd web && pnpm check-voice

# Markdown link-rot gate (F-705). Offline lane only — mirrors the
# `internal-links` job in .github/workflows/link-check.yml. The
# nightly external sweep is CI-only because it needs network and
# scheduled cadence.
check-links:
    lychee --offline --no-progress 'docs/**/*.md' 'crates/**/README.md' 'web/packages/**/README.md' README.md CHANGELOG.md AGENTS.md

# DESIGN.md lane: validates the design-system spec against the Stitch
# design.md schema (https://github.com/google-labs-code/design.md). Runs
# through scripts/lint-design.mjs so the contrast floor is the locked
# status-bar ratio (3.37:1) rather than the upstream WCAG AA default.
check-design:
    node scripts/lint-design.mjs

# All lanes.
check: check-rust check-web check-design

# Rust test suite.
test-rust:
    cargo test --all

# Serial `#[ignore]`-gated Rust tests that can't share a process-wide
# resource with parallel test binaries. Currently: the F-154 McpManager
# subprocess integration test, which conflicts with tokio's single
# process-wide SIGCHLD reaper when other test binaries are also spawning
# and reaping children in parallel. Run single-threaded, single-binary.
test-rust-serial:
    cargo test -p forge-mcp -- --ignored --test-threads=1

# Tauri webview integration tests. Gated on the `webview-test` feature
# because `tauri::test::mock_builder` pulls in the full Tauri runtime; keeping
# it off `test-rust`'s default build lets hosts without WebKitGTK headers
# still run the pure-Rust suite. Covers:
#   - forge-shell/tests/ipc_*.rs           (F-020 / F-051 / F-068 / F-069 / F-125)
#   - forge-shell/tests/approval_commands.rs (F-036)
test-rust-webview:
    cargo test -p forge-shell --features webview-test

# Web test suite.
test-web:
    cd web && pnpm -r test

# Both lanes.
test: test-rust test-rust-serial test-rust-webview test-web

# Regenerate the TypeScript bindings from Rust types. ts-rs emits each
# `#[ts(export)]` type to `web/packages/ipc/src/generated/` as a side effect
# of the auto-generated `export_bindings_*` test cases — `cargo build` does
# NOT trigger export, only `cargo test`. Run this after editing any Rust
# type that derives `TS`, then commit the regenerated files.
generate-ts:
    cargo test --workspace --quiet --tests export_bindings_

# Verify committed TS bindings match what ts-rs would regenerate from the
# current Rust sources. Self-contained: regenerates first, then diffs. CI
# wires this in as a drift gate — see .github/workflows/ci.yml.
ts-check: generate-ts
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
