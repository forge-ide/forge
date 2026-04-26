# Phase 2 Playwright UATs

Spec stubs for the Phase 2 UAT plan (`docs/testing/phase2-uat.md`). Each `uat-NN-<slug>.spec.ts` is currently `test.skip(...)` with a human-readable reason — see the table below for what blocks each spec from running.

The harness invokes this directory via `pnpm run test:e2e:phase2`. As specs are implemented, remove their `test.skip(...)` calls.

| Spec | UAT plan section | Currently blocked on |
|------|------------------|----------------------|
| `uat-01-outcome-gate.spec.ts` | UAT-01 | `tauri-driver` real-shell harness; `data-testid="drop-zone-{zone}"` for in-flight drag visual feedback (per plan §UAT-01 instrumentation gap) |
| `uat-02-editor-pane.spec.ts` | UAT-02 | `tauri-driver`; iframe-internal Monaco diagnostic-line introspection (plan §UAT-02 instrumentation gap) |
| `uat-03-terminal-pane.spec.ts` | UAT-03 | `tauri-driver`; xterm.js cell-level DOM scrape (plan §UAT-03 instrumentation gap) |
| `uat-05-agents-sub-agents.spec.ts` | UAT-05 | `tauri-driver`; mocked-provider system-prompt echo for AGENTS.md injection assertion; `data-testid="agent-source"` (plan §UAT-05 instrumentation gap) |
| `uat-06-agent-monitor.spec.ts` | UAT-06 | `tauri-driver`; promote-to-foreground affordance not yet shipped (plan §UAT-06 instrumentation gap) |
| `uat-07-context-picker.spec.ts` | UAT-07 | `tauri-driver`; `data-testid="picker-truncation-notice"` for inline truncation row (plan §UAT-07 instrumentation gap) |
| `uat-08-rerun-replace.spec.ts` | UAT-08 | `tauri-driver`; mocked re-run-aware provider script; `data-testid="message-branch-action"` (plan §UAT-08 instrumentation gap) |
| `uat-09-state-coverage.spec.ts` | UAT-09 | Mocked-IPC fixtures for state induction; `data-testid="terminal-pane-error"` (plan §UAT-09 instrumentation gap) |
| `uat-11-command-palette.spec.ts` | UAT-11 | `tauri-driver`; full command-registry harness |
| `uat-12-settings-approvals.spec.ts` | UAT-12 | `tauri-driver`; tempdir workspace + atomic-write spy |

UATs **04** (MCP import) and **10** (security gates) are bash-driven only — see `docs/testing/phase2-uat.sh`.
