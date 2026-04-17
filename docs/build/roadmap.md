# Roadmap

> Extracted from IMPLEMENTATION.md §12-13 — Phase 0-4, first sprint tickets F-000 to F-017, sprint totals, and definition of done

---

## 12. Phased roadmap

The sequencing that builds toward v1.0. Milestones are outcome-based.

### Phase 0 — Foundations (weeks 1–2)
**Outcome.** A new user can run `forge session new` and get a working session process with IPC.
- Repo scaffold, workspace, CI, rustfmt/clippy baselines
- `forge-core`, `forge-ipc` skeletons with `ts-rs` working
- `forge-session` bin: UDS handshake, event log append with schema header, mock provider chat round-trip
- `forge-cli`: `session new agent`, `session new provider`, `session list`, `session tail`, `session kill`
- `workspaces.toml` and `meta.toml` readers/writers
- **Ships**: a CLI-only preview; no GUI.

### Phase 1 — Single provider, minimal GUI (weeks 3–6)
**Outcome.** A user can open Forge, start a session, chat with Anthropic, and see tool calls.
- `forge-providers::AnthropicProvider` with SSE streaming
- `forge-shell` Tauri bin + webview bootstrap (Solid app)
- Dashboard view (sessions list + providers) — both active and archived filters
- Session window with single chat pane (layout system exists but splits come later)
- Tool call cards, four-scope inline approval UI
- File read/write tool implementations, process isolation level 1
- Session archive on end for `persist` sessions

### Phase 2 — Full layout, MCP, agents (weeks 7–10)
**Outcome.** Full session surface; external capabilities via MCP; agents spawn sub-agents and run in background.
- Pane layout: splits H/V/grid, drag-to-dock, minimum-width adaptations
- Editor pane with Monaco in iframe, `monaco-languageclient` LSP
- Terminal pane with `forge-term` (ghostty-vt) and xterm.js-compatible rendering
- Files sidebar toggle
- `forge-mcp` with stdio + http transport, universal-standard `.mcp.json` schema, `forge mcp import`
- `forge-agents` with `.agents/*.md` parsing, `AGENTS.md` auto-injection, orchestrator, sub-agent banners
- Background agents
- Agent monitor view
- @-context picker
- Re-run (Replace variant; Branch variant scaffolded per SPECS.md §15)

### Phase 3 — Breadth (weeks 11–14)
**Outcome.** Multi-provider, skills, catalog, usage, containers.
- OpenAI, Ollama, custom OpenAI-compat providers
- Skills loader (agentskills.io format, `forge skill install` from Git URL + local path)
- Catalog view (skills/MCP/agents manager with scope-aware entries)
- Usage view with charts, limits table, and cross-workspace aggregation (strategy decided here)
- `forge-oci` container support; Level 2 sandbox
- Context compaction (auto at 98%, user-invoked anytime)
- Parallel read-only tool calls
- Re-run Branch UI
- Opt-in cross-session memory

### Phase 4 — Polish and v1.0 (weeks 15–18)
**Outcome.** Ships.
- LSP bootstrap for the full 16 default servers
- Markdown preview pane (pulldown-cmark renderer), sticky toggles
- Built-in static preview server (`forge preview`)
- Command palette (full scope)
- Settings UI
- Install packages for macOS and Linux (dmg, deb, appimage)
- Windows install documentation for WSL2 path
- Public docs site
- Telemetry (opt-in, local-first)
- Tabbed-window mode (non-default)

### v1.1+ (post-ship)
- Image attachments in composer
- Browser OAuth provider login
- Remote sessions (`forge session attach ssh://...`)
- HTTP tarball skill distribution
- Plugin marketplace

### v1.3
- Native Windows build (named pipes IPC, job-object sandboxing)

---

## 13. First sprint (2 weeks)

Concrete, ticket-sized tasks for the first 10 working days. Assumes one engineer for scope sizing; a team of 3 could parallelize heavily.

### Sprint goal
A CLI-only preview where `forge session new agent default` spawns a real session process, `forge session tail` streams its output, and the mock provider responds with a scripted multi-turn exchange including one tool call. All persistence is filesystem-only (no SQLite).

### Tickets

**F-000 · Repo reset: retire the VS Code fork · 0.5 day**
The `forge-ide/forge` repo currently holds a VS Code fork. That codebase is incompatible with everything we're building — different language, different architecture, different product. This ticket wipes the slate.

Steps:
1. **Tag and archive the old.** Push a `legacy-vscode-fork` tag at the current `main` HEAD. Create a `legacy/vscode` branch from the same commit as a permanent preservation path. These stay in the repo forever — retrievable but never merged back.
2. **Create a preservation branch for the old `main`.** Push the existing `main` to `archive/main-pre-rewrite` before any destructive change. Belt-and-suspenders alongside the tag.
3. **Clear `main`.** `git checkout --orphan rewrite-main`, delete all tracked files, commit as `chore: reset for v2 rewrite (see legacy-vscode-fork tag)`. Force-push to `main` (coordinate with anyone else who may have clones — in practice: confirm with the handful of people who have the repo). Delete all other branches that descend from the old fork.
4. **Seed the new `main`.** Initial commit contains only: `README.md` (one paragraph: "Forge is being rewritten as a Rust+Tauri native workshop for agentic work. See docs/CONCEPT.md. Legacy VS Code fork preserved at tag `legacy-vscode-fork`."), `LICENSE`, `.gitignore`, and the three docs (`docs/CONCEPT.md`, `docs/SPECS.md`, `docs/IMPLEMENTATION.md`, `docs/DESIGN.md` copied from the design repo).
5. **Close the old issue tracker lane.** Move any issues referencing the VS Code fork to a `legacy` milestone and close them; they can be reopened if relevant. Open fresh issues for F-001 through F-017 so the board reflects the new plan.
6. **Update the repo description and homepage** on GitHub to match the new product framing. The old description probably reads "A fork of VS Code with AI features" or similar — replace with "Native desktop workshop for agentic work. Any AI. One editor. Transparent by default."
7. **Notify.** Post a `DISCUSSIONS.md` note or GitHub Discussion explaining the rewrite, linking to the legacy tag for anyone who needs it, and pointing at the new docs.

**Definition of done.** `git log main` shows 1 commit. `git tag` shows `legacy-vscode-fork`. `git branch -a` shows `main`, `archive/main-pre-rewrite`, `legacy/vscode`. Repo homepage reflects the new framing. Three docs live at `docs/`. No Electron, no TypeScript-at-scale, no VS Code source files on `main`.

**F-001 · Repo & workspace scaffold · 0.5 day**
Building on F-000's clean `main`. Create cargo workspace with all crate folders (empty libs). Add `rust-toolchain.toml` (stable). Add `.editorconfig`, dogfooded `AGENTS.md`, `.mcp.json`, `.agents/default.md` in repo root. CI: `cargo check --all-targets`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`. Define `ForgeError` in `forge-core`.

**F-002 · Core types in `forge-core` · 1 day**
Define: `SessionId`, `WorkspaceId`, `AgentId`, `ProviderId`, `MessageId`, `ToolCallId`, `AgentInstanceId`, `SessionPersistence`, `SessionState`, `RosterScope`, `ApprovalScope`. Derive `Serialize + Deserialize + TS`. Add test-based TS export that writes to `web/packages/ipc/src/generated/*.ts`. Verify CI runs the generation and diffs.

**F-003 · Event type + in-memory Transcript · 0.5 day**
Define `Event` variants exactly as in §3.1. Implement `Transcript::append`, `Transcript::from_file`. Round-trip test: write 100 events, read, compare.

**F-004 · JSONL event log persistence with schema header · 0.5 day**
Implement `.forge/sessions/<id>/events.jsonl` append. First line always `{"schema_version": 1}`. Reader verifies the header; refuses if absent. Writer uses `tokio::io::BufWriter` with explicit flush every 50ms.

**F-005 · `.forge/` setup and self-gitignore · 0.25 day**
On first write to `.forge/<ws>/`, create `.forge/.gitignore` containing `*` if it doesn't exist. Verify git-status on a workspace shows `.forge/` untracked then disappears.

**F-006 · `meta.toml` and `workspaces.toml` · 0.5 day**
Write/read the session metadata and user workspace registry. Include all fields from §7.2 and §7.3.

**F-007 · UDS server skeleton in `forge-session` · 1 day**
`forge-session` binary listens on `$XDG_RUNTIME_DIR/forge/sessions/<id>.sock` (fallback `/tmp/forge-<uid>/sessions/<id>.sock`). Accept connections, perform Hello/HelloAck handshake (including schema_version in HelloAck), reject unknown protos. Use `tokio::net::UnixListener`.

**F-008 · Length-prefixed JSON framing · 0.5 day**
A `FramedStream` in `forge-ipc` wrapping `tokio::net::UnixStream` with `Framed<_, LengthDelimitedCodec>` + JSON codec. Unit tests for round-trip of every session message kind.

**F-009 · Subscribe + Event replay · 1 day**
Implement `Subscribe { since }` handling in session: seek the JSONL log past the schema header to the seq, stream all events after to the client, then tail live. Test with a connection that subscribes mid-stream and receives both historical and live events.

**F-010 · Mock provider · 0.5 day**
`forge-providers::MockProvider` reads `~/.config/forge/mock.json` (configurable path) and streams pre-scripted chunks. A scripted turn looks like `{"delta":"Hello "}`, `{"delta":"world"}`, `{"tool_call":{"name":"fs.read","args":{"path":"README.md"}}}`, `{"done":"tool_use"}`.

**F-011 · Agent definition loader · 0.5 day**
Parse `.agents/<n>.md` and `~/.agents/<n>.md`. YAML frontmatter (`gray_matter` crate) + prose body. Reject `isolation: trusted` for user-defined agents. Workspace wins on name collision. `AGENTS.md` loaded from workspace root and cached for system prompt injection.

**F-012 · Session orchestrator loop · 1.5 days**
Session process, on `SendUserMessage`:
1. Append `UserMessage` event
2. Build `ChatRequest` (agent from `.agents/default.md` or bare provider from session meta) with AGENTS.md injected
3. Call provider's `chat()`, iterate stream
4. Append `AssistantMessage` (stream_finalised=false) on first chunk
5. Append `AssistantDelta` per chunk
6. On tool call: append `ToolCallStarted`, emit `ToolCallApprovalRequested` if not whitelisted, else execute
7. Append `ToolCallCompleted`, feed result back to provider
8. Append `AssistantMessage.stream_finalised=true` on stream end

**F-013 · fs.read tool + path validation · 0.5 day**
Agent-path-aware `fs.read` in `forge-fs`. Validate paths against agent's `allowed_paths` globs. Return content, bytes, sha256. Covered by the mock provider's scripted tool call.

**F-014 · Approval flow (sprint-minimum) · 0.5 day**
For this sprint: `--auto-approve-unsafe` flag auto-approves all tool calls. Otherwise, emit `ToolCallApprovalRequested` and wait. Full four-scope UI is phase 1 territory.

**F-015 · `forge-cli` binary · 1.5 days**
`clap`-based CLI. Commands for this sprint:
- `forge session new agent <n> --workspace PATH`: fork `forged`, print `session <id> started at <sock>`
- `forge session new provider <spec> --workspace PATH`: same, bare provider
- `forge session list`: scan `.forge/sessions/` in known workspaces, Hello-ping each for state
- `forge session tail <id>`: connect, subscribe since 0, pretty-print events to stdout
- `forge session kill <id>`: send kill signal
- `forge run agent <n> --input -`: one-shot ephemeral; spawn session, send single message from stdin, tail until done, exit with session's exit code

**F-016 · Integration test: full headless turn · 1 day**
In `tests/headless_session/basic.rs`: spawn `forged`, connect via UDS, send `SendUserMessage`, receive deltas + tool call + final message. Asserts full event sequence. Uses `MockProvider` with `mock.json` scripted responses. Runs in CI.

**F-017 · Docs: ADR-001 (UDS protocol) · 0.5 day**
Write `docs/architecture/ADR-001-session-uds-protocol.md` capturing the decisions from §5. Includes the schema header convention.

### Sprint totals
~12 engineer-days of tickets for a 10-working-day sprint. Intentional slack for refactoring and PR cycle time. If the team is 2+ engineers, F-005, F-006, F-010, F-013, F-017 parallelize easily. F-000 is the serial gate — nothing else starts until the repo is reset.

### Definition of done
1. `git log main` shows a clean history starting from the rewrite commit; `legacy-vscode-fork` tag exists and is reachable.
2. `cargo test --workspace` is green.
3. CI runs `tests/headless_session/basic.rs` and passes.
4. A human can run:
   ```
   $ forge session new agent default --workspace /tmp/acme
   session a3f1b2c4 started at /tmp/forge-1000/sessions/a3f1b2c4.sock
   $ forge session tail a3f1b2c4 &
   $ echo "refactor src/main.rs" | forge run agent default --input -
   ```
   …and see user message, assistant deltas, tool call, tool result, final assistant text.
5. `scripts/gen-ts-types.sh` runs clean; `web/packages/ipc/src/generated/*.ts` updated.
6. No SQLite or other database dependencies; `grep -r sqlite crates/` returns no matches.
7. No VS Code source artifacts on `main`; `grep -ri "microsoft/vscode" .` on the working tree returns no matches.
