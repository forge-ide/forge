## Scope

Cross-cutting supply-chain hygiene findings surfaced by Phase 1 scanners. Not single-vulnerability issues but process-level gaps that should close before Phase 2. Combining four items into one issue per maintainer decision.

## Findings bundle

### S1 — `cargo audit` and `cargo deny` not installed locally; not wired into CI

The Phase 1 audit had to install these tools. They are the standard Rust supply-chain scanners; they should run on every PR.

**Action:** add `cargo audit` and `cargo deny` invocations to the CI workflow (GitHub Actions). Fail on advisories `medium` or higher; warn on unmaintained. The Phase 1 audit left a minimal `deny.toml` at repo root — use it as the starting point and iterate.

### S2 — Workspace crates lack `license` field (12 crates unlicensed)

`cargo deny` flagged 12 crates as unlicensed: `forge-agents`, `forge-cli`, `forge-core`, `forge-fs`, `forge-ipc`, `forge-providers`, `forge-session`, `forge-shell`, plus workspace roots. None have a `license = "..."` field in their `Cargo.toml`.

**Action:** add `license = "MIT OR Apache-2.0"` (or the project's chosen license) to each workspace-member `Cargo.toml`. Add corresponding `LICENSE-MIT` / `LICENSE-APACHE` files at repo root.

### S3 — 17 unmaintained transitive dependencies (gtk-rs family, fxhash, unic-*)

From `cargo audit` and `cargo deny`:
- **gtk-rs GTK3 bindings** — 9 unmaintained advisories (Tauri 2 dep chain). Tauri's webkit2gtk-webview uses these.
- **fxhash** — `RUSTSEC-2025-0057` (unmaintained)
- **unic-char-property / unic-char-range / unic-common / unic-ucd-ident / unic-ucd-version** — 5 unmaintained
- **proc-macro-error** — unmaintained (build-time only)
- **`RUSTSEC-2024-0429`** — Unsoundness in `Iterator` impl for `glib::VariantStrIter` (webkit dep chain)
- **`RUSTSEC-2026-0097`** — `rand` unsound with a custom logger using `rand::rng()` (not exercised today)

**Action:** none immediately actionable from this repo; all are transitive through Tauri. Track by watching Tauri 2 release notes for updated gtk-rs / webkit bindings. Add all 9 gtk-rs advisories to `deny.toml`'s `[advisories] ignore = [...]` list with expiry dates, so the CI check remains green until upstream updates.

### S4 — `esbuild` and `vite` dev-server advisories

`pnpm audit` flagged 2 moderates:
- **GHSA-67mh-4wv8-2f99** (`esbuild ≤0.24.2`) — any website can send requests to the dev server
- **GHSA-vg6x-rcgg-rjx6** (`vite ≤6.4.1`) — path traversal in optimized-deps `.map` handling

Both are dev-server only; not a production vulnerability. Still worth addressing so `pnpm audit` stays green.

**Action:** `pnpm --filter app update esbuild vite` (or bump via the relevant direct deps — check `pnpm why esbuild`). Confirm `pnpm audit` returns 0 moderates.

## Definition of Done

- [ ] CI workflow runs `cargo audit` and `cargo deny check` on every PR; fails on medium+ advisories
- [ ] `deny.toml` accepts (with expiry dates) the 9 gtk-rs unmaintained advisories pending Tauri upstream updates
- [ ] All 12 workspace `Cargo.toml` files declare a `license` field
- [ ] `pnpm audit` returns 0 moderates after updating esbuild/vite
- [ ] Documentation (README or `docs/dev/security.md`) links to the scan results and explains the suppression policy

## References

- `/tmp/forge-audit-phase-1/scanners/` — raw scanner output captured during this audit
- `/home/jeroche/repos/forge/deny.toml` — seed configuration
- https://rustsec.org/
- https://docs.npmjs.com/cli/v10/commands/npm-audit
