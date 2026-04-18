# Phase 0 User Acceptance Test Plan

**Scope:** Phase 0: Foundations — CLI-only, no GUI  
**Outcome gate:** A user can spawn a session, send a message, approve a tool call, receive a response, and replay the transcript from disk.

---

## Prerequisites

| Item | Requirement |
|------|-------------|
| Build | `cargo build --workspace` succeeds |
| Binaries | `forge` and `forged` on `$PATH` or co-located |
| Mock agent | `.agents/test-agent.md` exists in the test workspace |
| Mock provider | `FORGE_PROVIDER_SCRIPT` set, or default mock provider active |
| Workspace | An empty temp directory (no pre-existing `.forge/`) |

---

## UAT-01: Spawn a Session

**Feature:** `forge session new agent`

| Step | Action | Expected |
|------|--------|----------|
| 1 | `forge session new agent test-agent --workspace /tmp/uat-ws` | Prints session ID and socket path; exits 0 |
| 2 | Check socket exists: `ls $FORGE_RUNTIME_DIR/forge/sessions/<id>.sock` | Socket file present |
| 3 | Check PID file exists: `ls $FORGE_RUNTIME_DIR/forge/sessions/<id>.pid` | PID file contains a valid integer |
| 4 | Check event log created: `head -1 /tmp/uat-ws/.forge/sessions/<id>/events.jsonl` | First line is `{"schema_version":1}` (no space after colon) |

**Failure criteria:** Command hangs >5 s; socket never appears; exit code non-zero.

---

## UAT-02: List Sessions

**Feature:** `forge session list`

| Step | Action | Expected |
|------|--------|----------|
| 1 | With a session running (from UAT-01) | |
| 2 | `forge session list` | Session ID appears in output; status is `active`; workspace path shown |
| 3 | Kill the forged process manually: `kill $(cat <pid-file>)` | |
| 4 | `forge session list` again | Same session ID shown; status is `stale` or session absent |

---

## UAT-03: Tail Event Stream (Replay + Live)

**Feature:** `forge session tail`

| Step | Action | Expected |
|------|--------|----------|
| 1 | With a running session that has events | |
| 2 | `forge session tail <id>` | Replays all historical events to stdout in order; each line is valid JSON with a `seq` field |
| 3 | While tail is running, send a message (see UAT-05) | New events appear on stdout in real time |
| 4 | Session ends | Tail exits 0 |

---

## UAT-04: Kill a Session

**Feature:** `forge session kill`

| Step | Action | Expected |
|------|--------|----------|
| 1 | With a running session | |
| 2 | `forge session kill <id>` | Exits 0 |
| 3 | Check PID file removed | PID file no longer exists |
| 4 | Verify process gone: `ps <pid>` | No such process |
| 5 | `forge session list` | Session absent or marked stale |

---

## UAT-05: Send a Message and Receive a Response

**Feature:** UDS `SendUserMessage` → event stream

| Step | Action | Expected |
|------|--------|----------|
| 1 | Attach to a running session via UDS (e.g., using `forge session tail` in background) | |
| 2 | Send a `SendUserMessage` frame: `{"t":"SendUserMessage","text":"Hello"}` | |
| 3 | Observe event stream | Events emitted in order: `UserMessage` → `AssistantMessage(open)` → one or more `AssistantDelta` → `AssistantMessage(final)` |
| 4 | Verify each event has a monotonically increasing `seq` | No gaps or duplicates |

---

## UAT-06: Tool Call Approval Gate

**Feature:** Approval flow for non-whitelisted tools

| Step | Action | Expected |
|------|--------|----------|
| 1 | Trigger a turn that invokes `fs.read` (or any tool) | `ToolCallStarted` event emitted; followed by `ToolCallApprovalRequested` |
| 2 | Observe stream is **blocked** — no further events | Stream paused; session waits |
| 3 | Send `ApproveToolCall { id, scope: Once }` frame | `ToolCallApproved { by: User }` emitted; tool executes |
| 4 | Observe remaining events | `ToolCallCompleted` → continuation `AssistantDelta`(s) → `AssistantMessage(final)` |
| 5 | Repeat trigger for same tool | `ToolCallApprovalRequested` fires again (scope was `Once`) |

---

## UAT-07: Tool Call Auto-Approve Mode

**Feature:** `--auto-approve-unsafe` flag

| Step | Action | Expected |
|------|--------|----------|
| 1 | `forge run agent test-agent --auto-approve-unsafe --input -` → pipe a message | Session runs headless; tool calls execute without waiting |
| 2 | Observe event stream | `ToolCallApproved { by: Auto }` present; no `ToolCallApprovalRequested` emitted |
| 3 | Exit code | 0 on successful session completion |

---

## UAT-08: Headless One-Shot Run

**Feature:** `forge run agent`

| Step | Action | Expected |
|------|--------|----------|
| 1 | `echo "List files in ." \| forge run agent test-agent --auto-approve-unsafe` | Streams events to stdout; exits 0 when `SessionEnded` received |
| 2 | `forge run agent test-agent --input ./prompt.txt` | Same as above; input read from file |
| 3 | `forge run agent no-such-agent` | Exits non-zero; error message mentions agent not found |

---

## UAT-09: Event Log Durability and Replay

**Feature:** Append-only log with schema header; replay on reconnect

| Step | Action | Expected |
|------|--------|----------|
| 1 | Run a session, exchange several messages, then kill it | Events on disk in `.forge/sessions/<id>/events.jsonl` |
| 2 | Inspect file | First line: `{"schema_version":1}`; subsequent lines are JSON event objects |
| 3 | `forge session tail <id>` on the dead session's log (or restart session) | All historical events replayed in `seq` order from log |
| 4 | Corrupt the schema header of a test log file; attempt to open | CLI or session process rejects the file with a clear error |
| 5 | Open a totally empty `.jsonl` file | Rejected with error: missing schema header |

---

## UAT-10: UDS Protocol Error Handling

**Feature:** Frame validation and proto negotiation

| Step | Action | Expected |
|------|--------|----------|
| 1 | Connect raw to UDS socket; send garbage bytes (not valid JSON) | Session closes connection; no crash |
| 2 | Connect; send `Hello` with `proto: 99` | Session responds with `Error` or closes; does not hang |
| 3 | Connect; send a frame claiming 8 MiB (exceeds 4 MiB limit) | Connection closed; session stays up for other clients |
| 4 | Connect; send valid `Hello` then disconnect without `Subscribe` | Session cleans up client slot without crashing |

---

## UAT-11: Multi-Client Attach

**Feature:** Multiple simultaneous clients on one session

| Step | Action | Expected |
|------|--------|----------|
| 1 | Start one session | |
| 2 | Open two separate `forge session tail <id>` processes | Both attach successfully |
| 3 | Send a message from one client | Both tails receive the same events with identical `seq` values |
| 4 | Disconnect one tail | Remaining tail continues receiving events uninterrupted |

---

## UAT-12: CLI Argument Validation

**Feature:** Argument parsing and exit codes

| Command | Expected |
|---------|----------|
| `forge session new agent` (no agent name) | Exits non-zero; usage printed |
| `forge session tail` (no session ID) | Exits non-zero; usage printed |
| `forge session kill` (no session ID) | Exits non-zero; usage printed |
| `forge bogus-command` | Exits non-zero; "unknown command" or help printed |
| `forge --help` | Usage printed; exits 0 |
| `forge session --help` | Session subcommand help; exits 0 |

---

## UAT-13: Workspace Isolation

**Feature:** `.forge/` scoped to workspace; `.gitignore` auto-created

| Step | Action | Expected |
|------|--------|----------|
| 1 | Start a session with `--workspace /tmp/ws-a` | `.forge/` created under `/tmp/ws-a/` only |
| 2 | Check gitignore: `cat /tmp/ws-a/.forge/.gitignore` | File exists; ignores session data patterns |
| 3 | Start second session with `--workspace /tmp/ws-b` | `.forge/` created under `/tmp/ws-b/`; no cross-contamination |
| 4 | Check existing gitignore (pre-create `/tmp/ws-a/.forge/.gitignore` with custom content) | Existing file preserved; not overwritten |

---

## Pass/Fail Criteria

| Result | Definition |
|--------|-----------|
| **Pass** | All steps in the test produce the expected outcome |
| **Fail** | Any step diverges from expected; process crashes; exit code wrong; data corrupt |
| **Blocked** | Prerequisites not met (binary missing, no mock provider) |

All UAT-01 through UAT-08 must pass before Phase 0 can be considered shippable. UAT-09 through UAT-13 are required for the stability bar.
