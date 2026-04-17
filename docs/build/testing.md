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
- `forge-core`, `forge-providers`, `forge-agents`, `forge-session`: 80% line coverage
- UI: smoke tests only in v1 (no snapshot tests — they rot)
