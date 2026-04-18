# Phase 1 UAT Setup

One-time setup needed before running `docs/testing/phase1-uat.sh` or the
Playwright specs in `web/packages/app/tests/phase1/`. The UAT plan itself is
`docs/testing/phase1-uat.md`.

## 1. Rust binaries

```bash
cargo build --workspace
# Or, for faster UAT runs:
cargo build --release -p forge-cli -p forge-session -p forge-shell
```

The harness looks in `target/release/` first, then `target/debug/`. Either
works. `forge-shell` is needed for the real-shell Playwright specs; the rest
of the suite (mocked IPC + disk-state UATs) only needs `forge` and `forged`.

## 2. pnpm workspace

```bash
cd web
pnpm install
pnpm --filter app build           # confirms the app still compiles
pnpm check-tokens                 # design-token drift gate (F-018) — script lives in web/package.json
```

`check-tokens` is defined on the root `web/package.json`, not on the `app`
workspace. Run it from `web/` (not from `web/packages/app/`).

If `pnpm` is missing: `corepack enable && corepack prepare pnpm@9.12.0 --activate`.

## 3. Playwright

```bash
cd web/packages/app
pnpm exec playwright install      # downloads Chromium; ~170MB
```

Install once per machine. The Playwright version pinned in `package.json`
(`@playwright/test ^1.48.0`) picks the browser build. System dependencies on
Linux:

```bash
sudo pnpm exec playwright install-deps   # one-time, distro-dependent
# or on Fedora specifically:
sudo dnf install -y alsa-lib nss atk cups-libs libdrm libxkbcommon \
  at-spi2-atk mesa-libgbm libXcomposite libXdamage libXrandr libXScrnSaver
```

## 4. Provider selection (F-038)

`forged` picks its provider from, in precedence order:

1. `--provider <spec>` flag (forwarded by `forge session new agent ... --provider <spec>` and `forge session new provider <spec>`)
2. `FORGE_PROVIDER` env var
3. `MockProvider` default (uses `FORGE_MOCK_SEQUENCE_FILE` if set, else default path)

Spec grammar:

| Spec | Effect |
|------|--------|
| `mock` | MockProvider |
| `ollama:<model>` | OllamaProvider against `OLLAMA_BASE_URL` (default `http://127.0.0.1:11434`), e.g. `ollama:qwen2.5:0.5b` |

Examples for the UAT harness:

```bash
# UAT-01a (default, MockProvider via FORGE_MOCK_SEQUENCE_FILE)
./docs/testing/phase1-uat.sh --cli-only

# UAT-01c (real Ollama)
FORGE_PROVIDER=ollama:qwen2.5:0.5b ./docs/testing/phase1-uat.sh --cli-only

# Direct: spawn a single session against Ollama
forge session new agent test-agent --provider ollama:qwen2.5:0.5b --workspace "$WS"
```

## 5. Ollama (for UAT-01b, UAT-01c, UAT-03, UAT-12 variant A)

Install, start, and pull one tiny model. The UAT only needs the Dashboard to
enumerate models — any model smaller than ~1 GB is fine.

```bash
# Fedora / general Linux
curl -fsSL https://ollama.com/install.sh | sh

# Start the daemon in the background (if it isn't already):
ollama serve &

# Pull a lightweight model. Options in size order:
ollama pull qwen2.5:0.5b          # ~400 MB, fastest
ollama pull llama3.2:1b           # ~1.3 GB, higher quality

# Smoke check:
curl http://127.0.0.1:11434/api/tags    # should list the pulled models
```

Stop the daemon when you want to exercise UAT-03's unreachable variant:

```bash
pkill ollama
```

## 6. tauri-driver (for real-shell Playwright specs, optional)

Required for specs marked `requires tauri-driver` (UAT-01a/01b/03/12B). Until
you wire a webdriverio harness, skip this step — the mocked-IPC specs cover
the bulk of Phase 1's UI surface.

```bash
cargo install tauri-driver --locked
# Linux also needs webkit2gtk-driver:
sudo dnf install -y webkit2gtk4.1-driver   # Fedora
```

## 7. Run the harness

```bash
# Full suite:
./docs/testing/phase1-uat.sh --build

# Just the CLI disk-state UATs (fastest, no browser):
./docs/testing/phase1-uat.sh --cli-only

# Just the Playwright GUI specs:
./docs/testing/phase1-uat.sh --gui-only

# Focus on one UAT:
./docs/testing/phase1-uat.sh --test UAT-09
./docs/testing/phase1-uat.sh --gui-only --test UAT-02

# Debug a Playwright failure interactively:
cd web/packages/app && pnpm run test:e2e:ui
```

## What to expect first run

- **CLI UATs** (UAT-09, UAT-10, UAT-13) should pass once the Rust build is
  green. They're fully automated and do not depend on Ollama or Playwright.
- **Playwright UATs**:
  - **Runnable now**: UAT-02 (sessions list), UAT-04 (window lifecycle),
    UAT-05 (streaming + composer), UAT-06 (tool call card — partial), UAT-07
    (approval UI — partial), UAT-01b UI contract.
  - **Skipped by design** until follow-up fixtures land: UAT-01a (needs a
    real `forged` bridge), UAT-03 (needs Ollama toggle harness), UAT-08
    (needs `forged` + tempdir workspace), UAT-11, UAT-12.
  - **Runnable with Ollama**: UAT-01c — `ollama pull qwen2.5:0.5b`, then
    pass `--provider ollama:qwen2.5:0.5b` (or `FORGE_PROVIDER=ollama:...`).
    The Playwright spec itself remains `test.skip` pending the real-shell
    tauri-driver harness; the Rust integration round-trip covers the wiring
    today (`cargo test -p forge-session --test provider_selection -- --ignored`).

Spec files carry `test.skip(...)` with a human-readable reason; the
`README.md` next to them maps every reason to its fix.
