# Phase 1 security audit — artifacts

This directory preserves the frozen-in-time artifacts from the Phase 1 security audit run on **2026-04-18** via the `forge-milestone-security-audit` skill. The audit output (findings, threat model, triage decisions) lives in GitHub Issues; the files here are the supporting raw material a future audit can diff against.

## Authoritative output

The audit's final decisions live in GitHub, not in this directory:

- **Consolidated report:** [issue #113 — F-071](https://github.com/forge-ide/forge/issues/113)
- **All finding issues** under the Phase 1 milestone labeled `type: security`:
  ```
  gh issue list --repo forge-ide/forge \
    --milestone "Phase 1: Single Provider + GUI" \
    --label "type: security"
  ```

When a finding gets fixed, its issue closes — **do not update the markdown files here**. This directory is a snapshot, not a living doc.

## Summary

- **Scope:** 7 Rust crates + `web/packages/app` + `web/packages/ipc` (current state, not a diff)
- **Baseline:** git tag `phase-0` (`a5d498bee`) → HEAD at audit time (`b3ba56ec0d`)
- **Findings:** 0 critical / 11 high / 12 medium / 6 low (30 total including 1 consolidated report)
- **F-numbers assigned:** F-042 through F-071
- **Biggest risk ruled out:** shell-command injection in `shell_exec` (would have been critical). The tool uses `Command::arg(...)` style, not a shell-string invocation.
- **Dominant pattern in findings:** trust boundaries are *implemented but not tightened* — approval scope silently collapsed to `Once`, `allowed_paths = "**"`, UDS socket mode `0755`, Tauri CSP `null`, no per-session authz.

## Directory layout

```
docs/audits/phase-1/
├── README.md                  # this file
├── created-issues.json        # short-id → F-number → GitHub issue # mapping
├── create.sh                  # bash script that created the issues (for record)
├── scanners/                  # raw scanner output at audit time
│   ├── cargo-audit.json       # 0 vulns, 17 unmaintained, 2 unsound
│   ├── cargo-deny.json        # empty (cargo-deny writes diagnostics to stderr)
│   ├── cargo-deny-stderr.log  # license + advisory + bans diagnostics
│   └── pnpm-audit.json        # 2 moderates (esbuild, vite) — dev-server only
└── findings/                  # source markdown for each issue body
    ├── H1.md … H11.md         # 11 HIGH findings
    ├── M1.md … M12.md         # 12 MEDIUM findings
    ├── L1.md … L5.md          # 5 LOW findings
    ├── S.md                   # 1 supply-chain hygiene bundled issue
    └── REPORT.md              # the consolidated F-071 report body
```

## Threat model (reference)

Derived in Step 2 of the audit skill from Phase 1's newly-introduced capabilities (Ollama streaming, Tauri webview, four-scope approval UI, forge-fs I/O, Level-1 sandbox, session archive, shell_exec).

High (LLM → side-effect boundaries):
- T1: Tool-approval bypass
- T2: Path traversal / `allowed_paths` bypass
- T3: Shell command injection in `shell_exec` (ceiling: critical — not found)
- T4: Sandbox escape
- T5: XSS / raw-HTML injection in webview

Medium:
- T6: UDS socket mode not 0600
- T7: IPC deserialization attacks
- T8: NDJSON stream parsing DoS
- T9: Session archive disclosure/integrity
- T10: Production panic on attacker-reachable path

Low / supply-chain:
- T11: Supply-chain new-dep risk
- T12: Raw-PID SIGTERM (forge-cli)

Explicitly out of scope for Phase 1 (deferred):
- Credential storage / auth → Phase 3
- TLS cert validation (Ollama is localhost HTTP)
- Multi-tenant authorization (single-user desktop)
- Container-level isolation → Phase 3

## How to read the finding files

Each `<ID>.md` in `findings/` is the exact body that was posted to the matching GitHub issue, using this structure:

1. **Scope** — which area, which threat class, why it matters in Phase 1 specifically
2. **Finding** — location (`path:line`), severity, threat class, grounded description
3. **Reproduction** — concrete PoC or untrusted-input path
4. **Remediation** — named-symbol fix
5. **References** — CWEs, advisories, docs
6. **Definition of Done** — checklist for the fix PR

The consolidated `REPORT.md` is the source of the F-071 issue body; it contains the full scoreboard, threat model, and per-area scope table.

## Scanner baselines

The scanner JSON files are the **Phase 1 baseline**. The Phase 2 (or later) security audit will produce a matching `docs/audits/phase-2/scanners/` and compare — a diff reveals new advisories, newly-unmaintained deps, and newly-introduced license or ban violations.

The seed `deny.toml` lives at the repo root. The supply-chain hygiene finding (F-070 / #112) tracks wiring `cargo audit` and `cargo deny check` into CI.

## Related

- Skill: `.claude/skills/forge-milestone-security-audit/SKILL.md`
- Milestone-completion workflow: `.claude/skills/forge-complete-milestone/SKILL.md`
- Threat-model reference used by this audit: defined inline in the consolidated report (F-071)
