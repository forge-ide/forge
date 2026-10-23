# Build plan

v0.1 in nine milestones, each a vertical slice an agent can build and verify on its own.

## Decisions and assumptions

Items marked **assumed** are defaults chosen so work can start; change them before the milestone that depends on them.

| Topic | Choice | Status |
| --- | --- | --- |
| Platforms | Linux (x86_64, aarch64) and macOS (Apple silicon) in v0.1; Windows later | Decided |
| Linux desktop | Wayland and X11, WebKitGTK 4.1 | Assumed |
| UI | Tauri 2, SolidJS, TypeScript, CodeMirror 6 | Decided |
| Rust | Stable, pinned in `rust-toolchain.toml`, edition 2024 | Assumed |
| Node | Current LTS pinned in `.nvmrc`; pnpm pinned via `packageManager` | Assumed |
| Task runner | `just` | Assumed |
| Sandbox, Linux | Light: Landlock + seccomp. Standard: bubblewrap | Decided |
| Sandbox, macOS | Light and Standard: Seatbelt profiles. Container: Podman machine | Assumed |
| Agents in v0.1 | Claude Code, Codex, Gemini CLI, all through ACP | Decided |
| Languages in v0.1 | Rust, TypeScript, Python, Go | Decided |
| Git hosting in v0.1 | Push the branch with the user's git credentials; PR creation is v0.2 | Assumed |
| GitHub auth (v0.2) | Reuse `gh auth token` if present, else OAuth device flow | Assumed |
| Auto mode in v0.1 | Rules only; classifier is v0.2 | Decided |
| License | Apache-2.0 | Assumed |
| Telemetry | None in v0.1 | Assumed |

**v0.1 scope limits:** one repo per project and one active session at a time, but every data structure and API is built for many, so v0.2 removes limits instead of rewriting.

## Milestones M0 to M4

M0 through M2 are strictly in order. After M2, M3 and M4 can run in parallel, and M7 can start any time after M1.

### M0: Repo skeleton and CI

- [ ] Cargo workspace with every crate from the component architecture as a stub with one passing test.
- [ ] pnpm workspace: `apps/desktop`, `packages/ui`, `packages/client`, `packages/editor`.
- [ ] Tauri app opens a window showing a placeholder Solid page.
- [ ] `justfile` with `build`, `test`, `lint`, `fmt`, `ci`, `dev`.
- [ ] CI on Linux and macOS runs `just ci`: rustfmt, clippy with warnings as errors, cargo tests, eslint, TypeScript check, vitest.
- [ ] `AGENTS.md`, `LICENSE`, `README.md` at the root.

**Done when:** `just ci` passes locally and in CI on both platforms, and `just dev` opens the app.

### M1: Daemon core and API

- [ ] `forged` listens on the Unix socket; `forge` CLI connects. Auto-start from the app if not running.
- [ ] `forge-api` types for the v0.1 method set, with TypeScript generated into `packages/client`.
- [ ] `server.hello`, `project.open`, `project.list`, `project.get`, `events.subscribe`, `events.query`.
- [ ] SQLite with migrations; `forge-config` loads and validates `forge.toml` and user config.
- [ ] UI shows the project list and connection status, and reconnects with `afterSeq`.

**Done when:** `forge project open .` returns the project; a test kills and restarts the client mid-stream and loses no events; a CI check fails if generated TypeScript is out of date.

### M2: Sessions, worktrees, and the fake agent

- [ ] `forge-git`: create, list, and remove worktrees and `forge/<slug>` branches.
- [ ] `forge-session`: the state machine from the component architecture, persisted in SQLite.
- [ ] `forge-acp`: ACP client handling file read/write, terminals, permission requests, and session updates.
- [ ] `forge-fake-agent`: an ACP agent that replays a YAML script of steps (see quality-and-safety.md).
- [ ] `forge-events`: JSONL log plus SQLite index; redaction hook point.
- [ ] `session.create`, `prompt`, `cancel`, `get`, `list`, `close`.

**Done when:** an integration test runs the fake agent to edit two files and run one command in a worktree, the session moves Provisioning → Ready → Working → Review, and every step is in the event log.

### M3: Sandbox

- [ ] `Sandbox` trait and tiers: Light and Standard on Linux, Seatbelt Light and Standard on macOS, opt-in Unsandboxed.
- [ ] Per-session egress proxy with allowlist and connection log.
- [ ] Read-only toolchain mounts; hidden `$HOME`.
- [ ] Secret injection as proxy headers from the OS keyring.

**Done when:** the escape suite (quality-and-safety.md) passes on both platforms for Light and Standard, and the fake agent's M2 test passes inside each tier.

### M4: Policy engine, guards, hooks, approvals

- [ ] `Action`, `Decision`, and the evaluation order from the component architecture.
- [ ] Layered TOML rules: managed, user, project, session.
- [ ] Guards: secret scan on edits and output, protected paths.
- [ ] Shell hook runtime for `tool.pre`, `tool.post`, `edit.pre`, `edit.post`, `task.done`.
- [ ] Modes: Ask, Accept edits, rules-only Auto, Full access.
- [ ] `approval.list`, `approval.decide`; ACP permission requests wait on them.

**Done when:** the policy golden tests pass, and an API test shows an Ask decision blocking the fake agent until `approval.decide` arrives.

## Milestones M5 to M8

### M5: Real agents

- [ ] Adapter registry: one manifest per agent with install command, version pin, auth check, and known quirks.
- [ ] Agent profiles for Claude Code, Codex, and Gemini CLI.
- [ ] Native shim that sets each agent's own permission mode so it defers to Forge's policy.
- [ ] `agent.list` with install and auth status; UI prompts to install or sign in.

**Done when:** each of the three agents completes the scripted smoke task ("add a function and a unit test") in each sample repo under Accept-edits mode, following the manual checklist in `docs/smoke.md`.

### M6: Review store and review pane

- [ ] Checkpoints on shadow refs after each turn; `session.rewind`.
- [ ] Changeset and per-file diffs; hunk-to-turn mapping.
- [ ] Threads, replies, resolve, outdated detection.
- [ ] Request changes as one structured agent message; per-thread replies.
- [ ] Changes since last review; per-hunk revert.
- [ ] Review pane UI (ui-screens.md).
- [ ] `review.ship` with `push-branch` and `merge-local`.

**Done when:** an end-to-end test drives the fake agent through change → comment → request changes → agent reply → thread resolved; the review pane meets the large-diff performance target on WebKitGTK and macOS.

### M7: Editor, LSP, and agent files

- [ ] File tree and editor tabs; `file.read`, `file.write` with conflict detection by content hash.
- [ ] `forge-lsp`: lazy per-worktree servers, idle shutdown, memory budget; `lsp.request` passthrough.
- [ ] CodeMirror `lsp-client` wired to the daemon for the four v0.1 languages.
- [ ] Markdown preview with Mermaid; agent-file detection and lint for `AGENTS.md` and `CLAUDE.md`.

**Done when:** go to definition, hover, and diagnostics work in each sample repo, and the lint test fixtures produce the expected warnings.

### M8: Session UI, timeline, and packaging

- [ ] Board (single session in v0.1), approvals inbox, session view with transcript and timeline.
- [ ] Rewind from the timeline; mode and sandbox tier switches with confirmations.
- [ ] Linux packages: AppImage, `.deb`, `.rpm`, and an Arch `.pkg.tar.zst` with AUR `-bin` and `-git` packages; `forged` systemd user unit.
- [ ] macOS: signed and notarized `.dmg`; `forged` launchd agent.

**Done when:** on a clean Linux machine and a clean Mac, install → open a sample repo → run a Claude Code task → review → ship works without a terminal. This is the v0.1 gate from overview.md.

## Handing it to an agent

1. **Put the spec in the repo.** It lives in `docs/spec/` and is committed with `AGENTS.md`. Agents read files, not links.
2. **One task per session.** Split each milestone into tasks of roughly 400 changed lines or less, each on its own branch with its own PR. Give the agent the task, the milestone's done-when check, and the relevant spec files.
3. **Tests first.** Ask the agent to write the done-when check as a failing test before implementing it.
4. **Interfaces are gated.** Any change to `forge-api`, config schemas, or SQLite migrations needs your explicit approval in the PR. Everything else can be reviewed normally.
5. **Plan before code** for tasks touching more than two crates: have the agent post a plan and wait for approval.
6. **Keep the spec current.** A PR that changes behavior described in `docs/spec/` updates it in the same PR.
7. **Dogfood from M6.** Once the review pane works, build the rest of Forge inside Forge.
