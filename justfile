set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list

# Install JS dependencies from the lockfile.
install:
    pnpm install --frozen-lockfile

# Build the UI bundle; tauri-build needs it before any cargo build of the desktop crate.
ui-build: install
    pnpm --filter @forge/ui build

# Run the desktop app with hot reload. Starting forged lands in M1.
dev: install
    pnpm --filter @forge/desktop tauri dev

# Build everything in release mode.
build: ui-build
    cargo build --workspace --release

# Rust and TypeScript unit and integration tests.
test: ui-build
    cargo test --workspace
    pnpm test

# clippy (warnings as errors), eslint, tsc --noEmit.
lint: ui-build
    cargo clippy --workspace --all-targets -- -D warnings
    pnpm lint
    pnpm typecheck

# rustfmt and prettier.
fmt: install
    cargo fmt --all
    pnpm fmt

# Check formatting without writing.
fmt-check: install
    cargo fmt --all -- --check
    pnpm fmt:check

# Regenerate TypeScript types and JSON Schemas from Rust types.
gen:
    @echo "just gen: nothing to generate until forge-api has types (M1)"

# Fail if generated files are out of date. No-op until M1 adds generation.
gen-check: gen
    @echo "just gen-check: no generated files yet (M1)"

# Sandbox escape suite for this platform.
test-sandbox:
    @echo "just test-sandbox: the escape suite lands in M3" >&2
    @exit 1

# UI end-to-end tests with the fake agent.
test-e2e:
    @echo "just test-e2e: end-to-end tests land in M6" >&2
    @exit 1

# Everything CI runs.
ci: fmt-check lint test gen-check
