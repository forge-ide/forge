# ADR-004: `forge-core` Scope and the Persistence-Split Plan

**Status:** Accepted
**Date:** 2026-04-22

---

## Context

`crates/forge-core` is the declared dependency root for every other Forge crate. Its original charter (see `crate-architecture.md §3.1`) is narrow: shared types, ids, the `Event` enum, the `ForgeError` type, and cross-crate trait surface. Everything that depends on `forge-core` does so to agree on that vocabulary — not to inherit runtime I/O.

During Phase 2 the crate's surface area grew past that charter. `lib.rs` exposes nine `pub` modules with five additional private `mod` declarations (fourteen total), four of which carry significant TOML / JSONL persistence machinery:

| Module | Lines | What it owns |
|---|---|---|
| `settings.rs` | 702 | TOML atomic-write, deep-merge on raw `toml::Value`, dotted-key update (PR #299 / F-151) |
| `approvals.rs` | 252 | Two-tier TOML load/save with workspace-wins whole-entry merge (PR #272 / F-036) |
| `transcript.rs` | 359 | Event-stream filter + `apply_superseded` persistence helper |
| `event_log.rs` | 284 | Bounded line reader over `events.jsonl` |

Two further modules — `meta.rs` and `workspaces.rs` — also carry async `write_*` / `read_*` helpers, though at a smaller scale.

The drift matters because `forge-core` is the base of an eight-crate dependency graph. Every crate that only wants `ProviderId` or `Event` compiles against `tokio::fs`, `toml`, `ts-rs`, and the full persistence surface today. Thirty-five git log entries landed on the crate during Phase 2; of four files added in that window (`mcp_state.rs`, `runtime_dir.rs`, `settings.rs`, `approvals.rs`) only `mcp_state.rs` is a pure shared-type module. The "god-package" concern ceiling is being hit.

This ADR records the scope boundary `forge-core` is supposed to hold and the phased plan for restoring it. It is written now, before the split lands, so the next contributor who considers adding a persistence helper to `forge-core` has an explicit place to read "not here."

Cross-references: `crate-architecture.md §3.1` (current responsibilities), `persistence.md §7.7, §7.8` (approvals and settings schemas that will follow the split), `ADR-002` (the `McpStateEvent`-in-`forge-core` precedent for "shared types only").

_Inventory snapshot as of commit HEAD at PR submission; see `git log` for drift._

---

## Decisions

### 1. `forge-core` is shared types only — no filesystem I/O

**Decision.** `forge-core` owns the cross-crate type vocabulary and nothing else. Specifically:

| Category | Belongs in `forge-core` | Does not belong |
|---|---|---|
| Ids, newtypes | `WorkspaceId`, `SessionId`, `AgentId`, `ProviderId`, `ToolCallId`, `MessageId`, `AgentInstanceId`, `StepId`, `TerminalId` | — |
| Event model | `Event`, `EventKind`, `ContextRef`, `ApprovalPreview`, `ApprovalSource`, `EndReason` | `Transcript` fold helpers, `apply_superseded` |
| Error type | `ForgeError`, `Result` | — |
| Small shared enums | `SessionState`, `SessionPersistence`, `ApprovalScope`, `ApprovalLevel`, `CompactTrigger`, `StepKind`, `StepOutcome`, `TokenUsage`, `RerunVariant`, `RosterScope` | — |
| MCP lifecycle types | `McpStateEvent`, `ServerState` (per ADR-002) | — |
| Runtime-path helpers | `runtime_dir` (pure path composition, no I/O) | — |
| `Tool` trait shape | `tool.rs` | Concrete tool impls |
| Persistence records | `SessionMeta`, `WorkspaceEntry`, `ApprovalConfig`, `AppSettings` (as *types*) | The `read_*` / `write_*` helpers that operate on them |
| Filesystem I/O | — | TOML atomic-write, JSONL bounded reader, deep-merge, dotted-key update |

The distinction is load-bearing: shared types cross the IPC boundary (they are derived with `ts-rs` and appear on the wire); I/O helpers do not. A crate that only imports types should not compile `tokio::fs`.

**Rationale.** Every consumer crate pays the transitive-dependency tax of whatever ends up in `forge-core`. `forge-providers` depends on `forge-core` for the `Result` alias alone; it currently pulls `tokio::fs`, `toml`, and the full `settings` deep-merge path into its compile. Keeping `forge-core` to types restores the "depends on nothing Forge-specific, and carries no runtime I/O" invariant the original architecture documented.

The inverse — inlining types next to the I/O code that serialises them — is not viable either: `Event` must be constructible from any crate that emits one (the whole dependency graph), and the IPC boundary needs a single canonical definition. Types go up the stack; I/O goes down.

---

### 2. Persistence code moves to a sibling `forge-persist` crate

**Decision.** A new `crates/forge-persist` crate will own the four persistence modules currently in `forge-core`, plus the `meta` / `workspaces` I/O helpers:

```
forge-persist/
  src/
    lib.rs
    settings.rs       // moved from forge-core, unchanged API
    approvals.rs      // moved from forge-core, unchanged API
    event_log.rs      // moved from forge-core, unchanged API
    transcript.rs     // moved from forge-core, unchanged API
    meta.rs           // moved from forge-core
    workspaces.rs     // moved from forge-core
    atomic_write.rs   // consolidated tmp+rename helper (see §3)
```

`forge-persist` depends on `forge-core` for the types it serialises (`SessionMeta`, `AppSettings`, `ApprovalEntry`, `WorkspaceEntry`, `Event`) and for `ForgeError`. Nothing in `forge-core` depends back on `forge-persist`.

**Consumer topology after the move.**

| Crate | What it imports from `forge-persist` |
|---|---|
| `forge-session` | `event_log::EventLog`, `event_log::read_since`, `transcript::{Transcript, apply_superseded}`, `meta::{read_meta, write_meta}` |
| `forge-shell` | `settings::{AppSettings, save_raw_to_path, load_merged, apply_setting_update, user_settings_path_in, …}`, `approvals::{ApprovalConfig, ApprovalEntry, save_to_path, load_merged, …}`, `meta::read_meta`, `workspaces::{read_workspaces, write_workspaces}` |
| everyone else | nothing |

**Rationale.** Two crates touch persistence helpers directly: `forge-session` (event log, transcript folding, session metadata) and `forge-shell` (user-facing config — approvals, settings, workspace registry). The six other consumers of `forge-core` want none of it. Moving the modules into a sibling crate keeps the dependency fan-out identical to today for the two consumers that need persistence and *narrows* it for everyone else.

The alternative — splitting further (e.g. `forge-settings`, `forge-approvals`, `forge-event-log`) — is rejected for v1. The modules share an atomic-write invariant, a TOML-as-config idiom, and a two-tier user-vs-workspace merge pattern; colocating them means one place owns that invariant rather than three. Finer splits are revisitable later if one of the modules grows teeth the others do not share.

---

### 3. The atomic-write pattern is consolidated before the move

**Decision.** Before `forge-persist` is carved out, a single `atomic_write(path, bytes)` helper (tmp-file + rename, with parent-dir creation) replaces the three hand-rolled copies currently in `approvals::save_to_path`, `settings::save_raw_to_path`, and `meta::write_meta`. The helper lives in the same crate that will become `forge-persist` (initially added inside `forge-core` to minimise the diff), and all three callers route through it. Layouts — fixed in F-363 — join the same path.

**Rationale.** The persistence modules each grew their own copy of "stage to `<path>.tmp`, `fs::rename`, create parent dir if needed." F-363 already flagged and fixed one of the drift instances (`write_layouts` had diverged). Consolidating the pattern *before* the crate split means the move is a file rename, not a behavioural change — each caller already trusts a shared helper, so the only thing that changes at split time is the path the helper lives under.

This is also the right time to pin the invariant in docs (`persistence.md §7.7` and `§7.8` already reference the pattern; they will be retargeted to the consolidated helper). Subsequent additions of persistence code in the codebase should reach for `atomic_write` rather than rolling a fourth copy.

---

### 4. The split is phased; deferred beyond this ADR

**Decision.** This ADR records the intent and the target shape. The mechanical move happens in a later milestone, not in the PR that lands this document. The immediate value of the ADR is that the scope boundary in §1 becomes enforceable *today* — any new persistence code landing in `forge-core` fails review against the decision here, regardless of whether `forge-persist` has been created yet.

The sequencing, when the move is scheduled:

1. Consolidate the atomic-write helper inside `forge-core` (per §3). PR-sized.
2. Create `crates/forge-persist` with an empty `lib.rs` and a workspace-level `Cargo.toml` entry. PR-sized.
3. Move one module at a time (`settings` first — it is the largest and has the most tests to exercise), re-export from `forge-core` to avoid breaking callers mid-flight. PR-sized per module.
4. Flip each consumer from `forge_core::settings::*` to `forge_persist::settings::*`, remove the re-export. PR-sized per consumer.
5. Repeat for `approvals`, `event_log`, `transcript`, `meta`, `workspaces`.
6. Delete the empty shims in `forge-core`.

Each step compiles the workspace and runs the full test suite green before landing. A caller-facing break only happens at step 4 — and it is a pure import rewrite, no behaviour change.

**Rationale.** Large moves that straddle eight dependent crates are not viable as a single PR: the diff is unreviewable and the blast radius on CI is too large. Splitting into re-export-gated steps lets each piece land independently, keeps the build green at every step, and keeps the "one concern per PR" convention the Forge contribution guide assumes. It also matches how F-155 successfully moved `McpStateEvent` / `ServerState` across the `forge-core` / `forge-mcp` boundary (see ADR-002 §1): types moved first, re-export preserved the old import path, and downstream rewrites followed.

---

## Consequences

- `forge-core`'s charter is now explicit. Any PR that adds filesystem I/O, async helpers, or config-file machinery to `forge-core` should be redirected to `forge-persist` (once it exists) or held until the split lands. The scope boundary in §1 is the review-time checklist.
- The crate-architecture doc (`crate-architecture.md §3.1`) gains a "Scope boundary" note linking here, so a reader looking at the module list understands which entries are in-charter and which are transient.
- Downstream consumers do not move today. `forge_core::settings::*` and `forge_core::approvals::*` continue to work. The consumer-topology table in §2 is the *target* — it becomes reality after step 4 of the phased plan.
- `forge-providers` and the other pure-type consumers keep paying the persistence-compile tax until the split lands. This is intentional: solving the god-package problem by deleting features is not on the table, and the phased move is the cheapest path to the charter `forge-core` is supposed to hold.
- A later milestone may add an `Arc<dyn ConfigFile>`-style trait above the consolidated atomic-write helper — the natural next step once the modules share a home. Out of scope for this ADR; noted as a direction, not a commitment.
- If multi-workspace or multi-tenant daemons ever ship (see ADR-002 §2 consequences), `forge-persist` is where path-scoped persistence handles would live. Recording the crate now gives that future work a place to land.
