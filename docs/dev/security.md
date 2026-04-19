# Supply-chain security

Forge runs three scanners on every PR via `.github/workflows/ci.yml`:

| Scanner | Scope | Fails CI when |
|---|---|---|
| `cargo audit` (`rustsec/audit-check@v2`) | Rust vulnerability advisories (RUSTSEC) | any CVE advisory matches a dep in `Cargo.lock` |
| `cargo deny check` (`EmbarkStudios/cargo-deny-action@v2`) | licenses, bans, advisories, sources | any rule in [`deny.toml`](../../deny.toml) violates, including expired suppressions |
| `pnpm audit --audit-level moderate` | npm advisories for the web workspace | any moderate-or-higher advisory applies to `web/pnpm-lock.yaml` |

## Baselines

The Phase 1 baseline scanner output is frozen at [`docs/audits/phase-1/scanners/`](../audits/phase-1/scanners/):

- `cargo-audit.json` — 0 vulns, 17 unmaintained, 2 unsound
- `cargo-deny-stderr.log` — license + advisory + bans diagnostics
- `pnpm-audit.json` — 2 moderates (esbuild, vite dev-server; both resolved in F-070)

Each phase's security audit (see `.claude/skills/forge-milestone-security-audit/`) produces a matching `docs/audits/phase-N/scanners/` directory. A diff between consecutive phases reveals new advisories, newly-unmaintained deps, and newly-introduced license or ban violations.

## Suppression policy

The `[advisories] ignore` list in [`deny.toml`](../../deny.toml) is the single place where known advisories are suppressed. Every entry:

1. Cites the specific `RUSTSEC-YYYY-NNNN` ID.
2. Carries a **`[expires YYYY-MM-DD]`** marker in the `reason` field, roughly six months out.
3. Explains **why** the advisory is not actionable from this repo — typically a transitive dep (e.g. gtk-rs GTK3 bindings arrive via Tauri 2's `webkit2gtk-webview` chain and cannot be upgraded independently).

The expiry marker is a **reviewer-obligation cue, not an enforced rule**. `cargo-deny 0.19` does not yet have a native expiry check on advisory ignores, so the date is embedded in the reason string. When the date passes, the suppression keeps working until a human reviews it — the CI does not fail. Grep `deny.toml` for `expires 2026` (or whatever current year) ahead of each milestone to find entries due for reassessment. Extensions require either (a) fresh evidence the upstream fix is still out of reach, or (b) a direct upgrade path that this repo now controls.

Unsound advisories (e.g. `RUSTSEC-2024-0429` on `glib::VariantStrIter`, `RUSTSEC-2026-0097` on `rand::rng()` under a custom logger) are suppressed only after confirming Forge code does not exercise the unsafe path. The rationale is recorded inline in `deny.toml`.

## Licensing

All workspace crates declare `license = "MIT OR Apache-2.0"` and the repo root ships both [`LICENSE-MIT`](../../LICENSE-MIT) and [`LICENSE-APACHE-2.0`](../../LICENSE-APACHE-2.0). `cargo deny check licenses` enforces the allowlist in `[licenses]`; transitive deps under a license not listed there fail the check.

## Triggering an out-of-band scan

```bash
cargo audit
cargo deny check
( cd web && pnpm audit --audit-level moderate )
```

Pushing to any branch or opening a PR runs all three in CI.
