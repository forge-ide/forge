set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list

# Install JS dependencies from the lockfile.
install:
    pnpm install --frozen-lockfile

# Build the UI bundle; tauri-build needs it before any cargo build of the desktop crate.
ui-build: install
    pnpm --filter @forge/ui build

# Run the desktop app with hot reload. The app starts the freshly built forged if none is running.
dev: install
    cargo build -p forged
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

# Regenerate the desktop icon set from apps/desktop/assets/icon-source.png.
icons: install
    pnpm --filter @forge/desktop tauri icon assets/icon-source.png -o src-tauri/icons
    rm -rf apps/desktop/src-tauri/icons/{android,ios,Square*.png,StoreLogo.png,64x64.png}

# Regenerate TypeScript types and JSON Schemas from Rust types.
gen: (_gen-into "packages/client/src/generated" "docs/schemas")

# Fail if the generated files differ from what the Rust types produce.
gen-check: (_gen-into "target/gen-check/client" "target/gen-check/schemas")
    @diff -ru target/gen-check/client packages/client/src/generated \
        && diff -ru target/gen-check/schemas docs/schemas \
        || { echo "Generated files are out of date: run 'just gen' and commit the result." >&2; exit 1; }

_gen-into client schemas: install
    rm -rf {{schemas}} && mkdir -p {{client}} {{schemas}}
    cargo run --quiet -p forge-api --bin forge-api-gen -- {{client}}/api.ts
    cargo run --quiet -p forge-config --bin forge-config-schemas -- {{schemas}}
    pnpm exec prettier --log-level warn --ignore-path /dev/null --write {{client}} {{schemas}}

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
