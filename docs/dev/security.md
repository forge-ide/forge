# Security reference

This document is the operator-facing reference for Forge's runtime and supply-chain protections. The full threat models, exploit walk-throughs, and audit-time evidence are immutable snapshots under [`docs/audits/phase-1/`](../audits/phase-1/) — this page distills the parts an operator needs to keep the protections healthy in production.

## UDS trust boundary (F-044, F-056)

Forge's session daemon (`forged`) listens on a Unix domain socket. Anyone who can `connect(2)` that socket can drive the session, approve tool calls, and read the event stream — there is no in-band authentication beyond filesystem permissions. Two protections keep that boundary tight:

**1. Mandatory `XDG_RUNTIME_DIR`, no `/tmp` fallback.** `crates/forge-session/src/socket_path.rs` refuses to resolve a socket path when `XDG_RUNTIME_DIR` is unset. The error operators will see on stderr begins:

> `forged refuses to start: XDG_RUNTIME_DIR is unset. This env var must point to a per-user 0o700 directory (systemd sets it to /run/user/<uid> automatically). Set it explicitly or use FORGE_SOCKET_PATH to override the socket location. (F-044 / H8)`

If you see that line, the daemon's environment is missing the variable systemd normally sets to `/run/user/<uid>`. Restore it (or pin the socket via `FORGE_SOCKET_PATH`) — do **not** patch the code to fall back to `/tmp`. The threat closed by this refusal is multiple local users sharing a world-accessible `/tmp/forge-{uid}/` directory; the previous resolver also read `UID` from a shell var that child processes don't inherit, so it could mint `/tmp/forge-0/` and let any local user steer the session.

**2. Bind-then-probe, plus `chmod 0o600`.** `crates/forge-session/src/server.rs::bind_uds_safely` never pre-unlinks the socket path. It calls `bind(2)` first; on `EADDRINUSE` it briefly probes the existing entry with `UnixStream::connect`. If the probe succeeds, another live daemon is serving — `forged` bails. If the probe fails, `forged` calls `symlink_metadata` (which does not follow symlinks) and only unlinks an entry whose file type is `socket`. Symlinks and regular files are refused outright. Immediately after `bind`, the socket is `chmod`'d to `0o600`, and the parent `forge/sessions` directory is forced to `0o700`. Regression coverage lives in `crates/forge-session/tests/uds_bind_symlink_race.rs` (symlink, dangling-symlink, and regular-file variants).

The pre-F-056 code was `if path.exists() { remove_file(path) }; bind(path)` — an attacker with write access to the parent directory could plant a symlink and watch the daemon unlink whichever target it pointed at.

## Sandbox resource limits (F-055)

Tool invocations that the user has approved are spawned through `crates/forge-session/src/sandbox.rs::SandboxedCommand`. In addition to environment scrubbing and a fresh process group (so `killpg` tears down the whole tree on shutdown), the sandbox calls `setrlimit(2)` from `pre_exec` for five resources. Defaults from `SandboxConfig::default`:

| Resource | Default | Threat mitigated |
|---|---|---|
| `RLIMIT_CPU` | 30 s | runaway CPU on a single tool call (SIGXCPU) |
| `RLIMIT_AS` | 512 MiB | address-space exhaustion / large allocations |
| `RLIMIT_NPROC` | 4096 | fork bombs (see caveat below) |
| `RLIMIT_NOFILE` | 256 | fd-table exhaustion |
| `RLIMIT_FSIZE` | 100 MiB | cat-to-disk attacks (SIGXFSZ on overflow) |

Soft and hard limits are set to the same value, so the child cannot raise them. The `rlimits_bound_child_via_setrlimit` test in the same module probes `/proc/self/limits` from inside the sandbox to confirm `pre_exec` actually applied them — that test is the load-bearing regression for F-055.

**NPROC caveat.** `RLIMIT_NPROC` is enforced per real-uid by the kernel, not per-sandbox. Because `forged` typically shares its uid with the user's desktop session (or CI's test harness), the 4096 cap is tuned to stop a fork bomb (which saturates any cap within milliseconds) while leaving headroom for the uid's baseline process count — it is **not** a per-sandbox process budget. Per-sandbox isolation requires cgroups; that work is tracked in [F-078](https://github.com/forge-ide/forge/issues/151).

## Webview Content Security Policy (F-050)

The Tauri webview enforces a restrictive CSP. The production source of truth is `crates/forge-shell/src-tauri/tauri.conf.json` under `app.security.csp`; `web/packages/app/index.html` carries an identical `<meta http-equiv="Content-Security-Policy">` so the Vite dev server and the Playwright harness — neither of which read `tauri.conf.json` — apply the same policy. **Both files must stay in sync.**

Current policy:

> `default-src 'self' ipc: http://ipc.localhost; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: asset: https://asset.localhost; connect-src 'self' ipc: http://ipc.localhost; frame-ancestors 'none'; base-uri 'self'; object-src 'none'`

Notable directives:

- **`script-src 'self'`** — no inline scripts, no `eval`, no remote script CDNs. All JS ships from the bundled web assets.
- **`connect-src 'self' ipc: http://ipc.localhost`** — XHR / `fetch` / WebSocket targets are limited to bundled assets and the Tauri IPC bridge. The webview cannot exfiltrate data to an external host.
- **`style-src 'self' 'unsafe-inline'`** — `'unsafe-inline'` is required for runtime style injection from the bundled CSS pipeline; remote stylesheets remain blocked.
- **`frame-ancestors 'none'` and `object-src 'none'`** — the app cannot be framed and cannot load plugins.

If a future feature embeds a third-party renderer (e.g. a Markdown sandbox iframe, a remote LSP UI, or a model-hosted preview), expect to add a narrow `frame-src` allow-list and likely a `connect-src` host. Update both `tauri.conf.json` and `index.html` in the same change and re-run the Playwright harness — silently mismatched policies were the entire premise of H9.

## Operator runbook cross-reference

- Full Phase 1 threat models, exploit walk-throughs, and audit evidence: [`docs/audits/phase-1/`](../audits/phase-1/) — start with `REPORT.md`, then individual `H#.md` / `M#.md` / `L#.md` findings. These files are immutable snapshots; they are not updated as code evolves.
- Tracking issues for follow-up hardening:
  - [F-077](https://github.com/forge-ide/forge/issues/150) — DoS ceiling semantics + per-session aggregate byte budget (current per-message NDJSON cap is documented but the aggregate is not yet enforced).
  - [F-078](https://github.com/forge-ide/forge/issues/151) — `RLIMIT_NPROC` uid-wide caveat above + cgroup-based per-sandbox PID limit.

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
