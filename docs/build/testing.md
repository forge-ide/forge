# Testing Strategy

> Extracted from IMPLEMENTATION.md §11 — unit, integration, smoke test levels, mock provider, approval testing, and coverage targets

---

## 11. Testing strategy

### 11.1 Levels
| Level | Where | What |
|---|---|---|
| Unit | Each crate | Pure logic; ~70% of tests live here |
| Integration (Rust) | `tests/` top-level | Full session process exercised over UDS |
| Integration (TS) | `web/apps/shell/tests/` | Playwright against Tauri dev build |
| Smoke (CI) | `tests/cli_smoke/` | Full install + `forge run` headless against a mock provider |

### 11.2 Mock provider
`forge-providers/src/mock.rs` is a deterministic provider that reads a `.json` script describing the turn-by-turn response. Used everywhere tests need to drive a session.

### 11.3 Approval testing
A `--auto-approve-unsafe` flag on the session binary (dev-only; refused if released binary) for scripted tests.

### 11.4 Coverage targets

Every first-class workspace crate carries an explicit target. Bands reflect how the crate is exercised, not how important it is.

| Crate | Target | Rationale |
|---|---|---|
| `forge-core` | 80% line | Pure logic, config, credential resolution |
| `forge-providers` | 80% line | Provider adapters + mock |
| `forge-agents` | 80% line | Agent orchestration logic |
| `forge-session` | 80% line | Session lifecycle, IPC server |
| `forge-mcp` | 80% line | MCP client/server surface (Phase 2) |
| `forge-lsp` | 80% line | LSP multiplexer (Phase 2) |
| `forge-fs` | 80% line | Workspace FS abstractions (Phase 2) |
| `forge-term` | 80% line | Terminal/PTY logic (Phase 2) |
| `forge-ipc` | Integration only | Protocol definitions; exercised end-to-end via `forge-session` + `forge-shell` IPC tests |
| `forge-oci` | Integration only | Container runtime glue; exercised via session-level tests |
| `forge-cli` | Integration only | Thin entrypoint; exercised via `tests/cli_smoke/` |
| `forge-shell` | Smoke only | Tauri host; exercised via `just test-rust-webview` and the web Playwright lane — no line-coverage number |

- UI: smoke tests only in v1 (no snapshot tests — they rot)
