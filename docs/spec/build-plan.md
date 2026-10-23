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
| Node | Node 24 LTS pinned in `.nvmrc`; pnpm pinned via `packageManager` | Decided |
| Task runner | `just` | Assumed |
| Sandbox, Linux | Light: Landlock + seccomp. Standard: bubblewrap | Decided |
| Sandbox, macOS | Light and Standard: one Seatbelt profile. Container: Podman machine | Decided (Seatbelt); Assumed (Container) |
| Agent commits | Through Forge's `git_commit` MCP tool, run by the daemon; the shared git dir is read-only inside the sandbox | Decided |
| Agents in v0.1 | Claude Code, Codex, Gemini CLI, all through ACP | Decided |
| Languages in v0.1 | Rust, TypeScript, Python, Go | Decided |
| Git hosting in v0.1 | Push the branch with the user's git credentials; PR creation is v0.2 | Assumed |
| GitHub auth (v0.2) | Reuse `gh auth token` if present, else OAuth device flow | Assumed |
| Auto mode in v0.1 | Rules only; classifier is v0.2 | Decided |
| License | Apache-2.0 | Decided |
| Telemetry | None in v0.1 | Assumed |

**v0.1 scope limits:** one repo per project and one active session at a time, but every data structure and API is built for many, so v0.2 removes limits instead of rewriting.

## Milestones M0 to M4

M0 through M2 are strictly in order. After M2, M3 and M4 can run in parallel, and M7 can start any time after M1.

### M0: Repo skeleton and CI

- [x] Cargo workspace with every crate from the component architecture as a stub with one passing test.
- [x] pnpm workspace: `apps/desktop`, `packages/ui`, `packages/client`, `packages/editor`.
- [x] Tauri app opens a window showing a placeholder Solid page.
- [x] `justfile` with `build`, `test`, `lint`, `fmt`, `ci`, `dev`.
- [x] CI on Linux and macOS runs `just ci`: rustfmt, clippy with warnings as errors, cargo tests, eslint, TypeScript check, vitest.
- [x] `AGENTS.md`, `LICENSE`, `README.md` at the root.

**Done when:** `just ci` passes locally and in CI on both platforms, and `just dev` opens the app.

### M1: Daemon core and API

- [x] `forged` listens on the Unix socket; `forge` CLI connects. Auto-start from the app if not running.
- [x] `forge-api` types for the v0.1 method set, with TypeScript generated into `packages/client`.
- [x] `server.hello`, `project.open`, `project.list`, `project.get`, `events.subscribe`, `events.query`.
- [x] SQLite with migrations; `forge-config` loads and validates `forge.toml` and user config.
- [x] UI shows the project list and connection status, and reconnects with `afterSeq` (client side only: the UI resubscribes with `afterSeq`, but the daemon has no events to replay until M2).

**Done when:** `forge project open .` returns the project, and a CI check fails if generated TypeScript is out of date. The lossless-reconnect test moved to M2, because no events exist until sessions do.

### M2: Sessions, worktrees, and the fake agent

- [x] `forge-git`: create, list, and remove worktrees in `<repo>/.worktrees/`, on branches named by the user's convention (`{type}/{slug}` by default); name collisions ask the user.
- [x] `forge-session`: the state machine from the component architecture, persisted in SQLite.
- [x] `forge-acp`: ACP client handling file read/write, terminals, permission requests, and session updates.
- [x] `forge-fake-agent`: an ACP agent that replays a YAML script of steps (see quality-and-safety.md).
- [x] `forge-events`: JSONL log plus SQLite index; redaction hook point.
- [x] `session.create`, `prompt`, `cancel`, `get`, `list`, `close`, and `session.branchPreview`.
- [x] Deferred from M1: `events.subscribe` replays from the event log after `afterSeq`.
- [x] Deferred from M1: connection limits in `forged` before events flow: a bounded per-connection outgoing queue that drops a client that stops reading, a WebSocket handshake timeout, and a cap on in-flight requests per connection.
- [x] Deferred from M1: a subscription that completes after its connection closed must be removed at once, not on the next failed send.
- [x] Deferred from M1: before creating a worktree, resolve each repo path from `forge.toml` and check it is still inside the project root. The config check is lexical, so a symlinked repo path can point outside the root.

**Done when:** an integration test runs the fake agent to edit two files and run one command in a worktree, the session moves Provisioning → Ready → Working → Review, and every step is in the event log; and a test kills and restarts a client mid-stream, resubscribes with `afterSeq`, and loses no events. Settled for that test: notification `seq` is a daemon-wide stream cursor, so one `afterSeq` covers every topic, including `board` and `project/<id>`, which span sessions; event `seq` stays per session.

**No sandbox yet:** until M3, sessions run only unsandboxed, and only for agent profiles that set `allow_unsandboxed = true`; any other tier is `SandboxUnavailable`.

### M3: Sandbox

- [x] `Sandbox` trait and tiers: Light and Standard on Linux, Seatbelt Light and Standard on macOS, opt-in Unsandboxed.
- [x] Per-session egress proxy with allowlist and connection log.
- [x] Read-only toolchain mounts; hidden `$HOME`.
- [x] Environment allowlist for every tier; worktree git metadata the daemon doesn't trust.
- [x] Sandbox plumbing for per-session daemon endpoints: the proxy now (a loopback port in Light and Seatbelt; a Unix socket bound into Standard, forwarded to a loopback port inside), reusable for Forge's MCP endpoint in M6.
- [x] Secret injection as proxy headers from the OS keyring.

**Done when:** the escape suite (quality-and-safety.md) passes on both platforms for Light and Standard, and the fake agent's M2 test passes inside each tier.

**Later:** cgroup limits for CPU, memory, and process count per session (they also close the gap where a process that leaves its process group outlives a Light or Seatbelt kill).

### M4: Policy engine, guards, hooks, approvals

- [x] `Action`, `Decision`, and the evaluation order from the component architecture.
- [ ] Layered TOML rules: managed, user, project, session.
- [ ] Guards: secret scan on edits and output, protected paths.
- [ ] Shell hook runtime for `tool.pre`, `tool.post`, `edit.pre`, `edit.post`, `task.done`.
- [ ] Modes: Ask, Accept edits, rules-only Auto, Full access.
- [ ] `approval.list`, `approval.decide`; ACP permission requests wait on them.
- [ ] `commit.pre` shell hook; default deny rules for manual git commands that write shared state (`git commit`, `git push`, and `git stash` and `git branch` except their read-only forms), with a reason pointing agents at `git_commit`.

**Done when:** the policy golden tests pass, and an API test shows an Ask decision blocking the fake agent until `approval.decide` arrives.

## Milestones M5 to M8

### M5: Real agents

- [ ] Adapter registry: one manifest per agent with install command, version pin, auth check, and known quirks.
- [ ] Agent profiles for Claude Code, Codex, and Gemini CLI.
- [ ] Native shim that sets each agent's own permission mode so it defers to Forge's policy.
- [ ] `agent.list` with install and auth status; UI prompts to install or sign in.
- [ ] Each agent's shim allows Forge's `git_commit` tool and tells the agent to commit with it; the smoke task checks a real agent commits through it.

**Done when:** each of the three agents completes the scripted smoke task ("add a function and a unit test") in each sample repo under Accept-edits mode, following the manual checklist in `docs/smoke.md`.

### M6: Review store and review pane

- [ ] Checkpoints on shadow refs after each turn; `session.rewind`.
- [ ] Changeset and per-file diffs; hunk-to-turn mapping.
- [ ] Threads, replies, resolve, outdated detection.
- [ ] Request changes as one structured agent message; per-thread replies.
- [ ] Changes since last review; per-hunk revert.
- [ ] Review pane UI (ui-screens.md).
- [ ] `review.ship` with `push-branch` and `merge-local`.
- [ ] Minimal per-session Forge MCP endpoint with `git_commit`, passed to the agent at ACP session start; the daemon commits on the session branch after policy and the `commit.pre` hook, sharing commit code with checkpoints and ship.

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
2. **One task per session.** Give the agent the task, the milestone's done-when check, and the relevant spec files. A whole milestone can be one task on one branch.
3. **Tests first.** Ask the agent to write the done-when check as a failing test before implementing it.
4. **Interfaces are gated.** Any change to `forge-api`, config schemas, or SQLite migrations needs your explicit approval in the PR. Everything else can be reviewed normally.
5. **Plan before code** for tasks touching more than two crates: have the agent post a plan and wait for approval.
6. **Keep the spec current.** A PR that changes behavior described in `docs/spec/` updates it in the same PR.
7. **Dogfood from M6.** Once the review pane works, build the rest of Forge inside Forge.
