# Forge

Forge is a desktop development environment for directing and reviewing coding agents. A Rust daemon (`forged`) owns sessions, git worktrees, sandboxes, policy, and the event log. A Tauri 2 + SolidJS app and a CLI (`forge`) are thin clients of the daemon's JSON-RPC API. Agents connect over ACP (Agent Client Protocol).

The full spec lives in `docs/spec/`. Read the parts relevant to your task before starting:

- `overview.md`: goals, principles, features, MVP scope
- `component-architecture.md`: how each crate works
- `build-plan.md`: milestones, decisions, done-when checks
- `interfaces-and-data.md`: API methods, events, config schemas, SQL schema
- `ui-screens.md`: wireframes, states, keybindings
- `quality-and-safety.md`: tests, performance targets, threat model, failure handling

If the spec and the code disagree, stop and ask. Don't silently pick one.

## Repo layout

```
crates/
  forged/            daemon binary; SQLite migrations in crates/forged/migrations/
  forge/             CLI binary
  forge-api/         JSON-RPC method and event types (source of truth for the wire format)
  forge-session/     session lifecycle and supervision
  forge-acp/         ACP client
  forge-fake-agent/  scripted ACP agent for tests
  forge-sandbox/     isolation backends (Linux: Landlock/seccomp, bubblewrap; macOS: Seatbelt)
  forge-policy/      rules, modes, guards
  forge-hooks/       hook runner
  forge-git/         worktrees, branches, shadow refs
  forge-review/      changesets, threads
  forge-lsp/         language server host
  forge-mcp/         MCP hub
  forge-events/      event log and index
  forge-config/      config loading and layering
apps/desktop/        Tauri 2 shell (no business logic)
packages/ui/         SolidJS app
packages/client/     generated TypeScript client (do not edit by hand)
packages/editor/     CodeMirror 6 extensions
docs/spec/           the spec
docs/schemas/        generated JSON Schemas (do not edit by hand)
tests/fixtures/      sample repos and fake-agent scripts
```

## Toolchain

- Rust stable, pinned in `rust-toolchain.toml`, edition 2024.
- Node LTS pinned in `.nvmrc`; pnpm pinned via `packageManager` in `package.json`.
- `just` for all tasks. Don't call cargo or pnpm directly in docs or CI; add a recipe.
- Linux needs WebKitGTK 4.1 dev packages and bubblewrap. Run `forge doctor` (once it exists) to check.

## Commands

| Command | What it does |
| --- | --- |
| `just dev` | Run `forged` and the desktop app with hot reload |
| `just build` | Build everything in release mode |
| `just test` | Rust and TypeScript unit and integration tests |
| `just test-sandbox` | Sandbox escape suite for this platform |
| `just test-e2e` | UI end-to-end tests with the fake agent |
| `just lint` | clippy (warnings as errors), eslint, `tsc --noEmit` |
| `just fmt` | rustfmt and prettier |
| `just gen` | Regenerate TypeScript types and JSON Schemas from Rust types |
| `just ci` | Everything CI runs; must pass before you say you're done |

## Rules

- **Run `just ci` before declaring any task complete.** Report the result. If something fails that you didn't touch, say so instead of hiding it.
- **Write the test first.** For each task, turn its done-when check into a failing test, then make it pass.
- **Interfaces need approval.** Don't change `crates/forge-api`, config schemas, or SQL migrations without saying so explicitly in your summary and PR description. Never edit an existing migration; add a new one.
- **Never edit generated files** (`packages/client/src/generated/`, `docs/schemas/`). Change the Rust types and run `just gen`.
- **Keep the spec current.** If your change alters behavior described in `docs/spec/`, update it in the same change.
- **Stay in scope.** One task per branch and PR. Note unrelated problems in your summary instead of fixing them.
- **No new dependencies** without a sentence in the PR on why, its license, and its maintenance status. Prefer what's already in the workspace.
- **Both platforms.** Code under `forge-sandbox` and anything touching paths, sockets, or processes must handle Linux and macOS. Use `cfg(target_os)` modules, not runtime checks scattered through logic. If you can only test one platform, say which.
- **Security defaults are not negotiable.** Never weaken a sandbox tier, widen a default allowlist, or make a fallback to a weaker tier silent, even to get a test passing.

## Code conventions

**Rust**

- Errors: `thiserror` in library crates, `anyhow` only in binaries. API errors map to the codes in `interfaces-and-data.md`.
- Async: Tokio. No blocking calls on the runtime; use `spawn_blocking` for git and filesystem-heavy work.
- Logging: `tracing` with structured fields (`session_id`, `tool_call_id`). Never log secrets or full file contents.
- IDs: prefixed ULIDs via the helpers in `forge-api` (`SessionId::new()`), never raw strings.
- Each crate exposes a trait for its main component so others can be tested with fakes.
- Serde: `#[serde(rename_all = "camelCase")]` on wire types; deny unknown fields on config types.

**TypeScript / Solid**

- Strict TypeScript. No `any`; use `unknown` and narrow.
- Components never import `@tauri-apps/*`. Go through `packages/ui/src/host/`.
- Daemon state lives in the stores under `packages/ui/src/stores/`, updated only from daemon events. Components don't call the API to mutate local state optimistically unless the store handles rollback.
- Use `@tanstack/solid-virtual` for any list that can exceed 100 items.
- Every view handles loading, empty, error, and disconnected states (see `ui-screens.md`).

**Commits and PRs**

- Conventional commits: `feat(review): ...`, `fix(sandbox): ...`, `test(policy): ...`.
- PR description: what changed, which milestone task it completes, how it was tested (with `just ci` output summary), and any interface changes called out at the top.

## Testing notes

- Use `forge-fake-agent` with a YAML script for anything involving sessions. Real agents are for manual smoke tests only (`docs/smoke.md`).
- Integration tests start a real `forged` against a temp dir with `FORGE_DATA_DIR` and `FORGE_RUNTIME_DIR` set. Never touch the real user data dir in tests.
- Policy changes need golden test updates (`cargo insta review`); explain each changed snapshot in the PR.
- Sandbox tests must assert that escapes fail, not just that normal work succeeds.

## When to stop and ask

- The spec is ambiguous or contradicts the code.
- The task needs an interface, schema, or migration change.
- A security default would have to change.
- Reviews are leading down numerous rabbit holes.
- You can't get `just ci` green after a reasonable attempt.
