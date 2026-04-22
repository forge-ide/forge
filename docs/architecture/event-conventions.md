# Event conventions

Cross-cutting rules for every `#[derive(Serialize, Deserialize)]` type that
crosses the IPC boundary as an event or event payload. New variants MUST
follow these rules. Pinned exceptions are listed in full — any deviation
outside that list is structural drift and will be rejected in review.

## 1. Timestamps

- **Field name**: `at`.
- **Type**: `chrono::DateTime<chrono::Utc>`.
- **Wire shape**: RFC3339 string (serde-default for `DateTime<Utc>`).

Every new event variant that carries a timestamp MUST use `at: DateTime<Utc>`.
`SystemTime`, `Instant`, or other clock types do not cross the boundary;
only the chrono wire shape is supported.

### Pinned exceptions

These two fields retain their specialized names because the `AgentMonitor`
webview and surrounding tests pin on them, and the churn to rename them
exceeds the benefit. Wire-shape tests in
`crates/forge-core/tests/event_wire_shape.rs` lock both.

| Variant                    | Field         | Reason                                                   |
|----------------------------|---------------|----------------------------------------------------------|
| `Event::StepStarted`       | `started_at`  | Paired with `StepFinished.duration_ms` — the frontend's  |
|                            |               | `AgentMonitor.tsx` distinguishes step-open from other    |
|                            |               | event timestamps via the field name.                     |
| `Event::ResourceSample`    | `sampled_at`  | Emphasizes sample-vs-emit distinction (the monitor may   |
|                            |               | batch samples before emitting). The AgentMonitor         |
|                            |               | webview reads this field by name.                        |

Any future rename of either exception requires a synchronized change to
the webview, the ts-rs bindings, and a follow-up ticket referencing F-380.

### Variants intentionally without a timestamp

Some events do not carry their own `at` because they are strictly
correlative — they reference an already-stamped event by id, and
downstream consumers derive ordering from the referenced event's
timestamp. Documented per variant:

- `BranchSelected`, `BranchDeleted`, `MessageSuperseded` — id-only
  markers; the superseded/selected message already has `at`.
- `SubAgentSpawned`, `BackgroundAgentStarted` vs `UsageTick` — `UsageTick`
  is a periodic tick (not a transition); today it's stamped via the event
  log's own append ordering and surfaces no timestamp of its own.
- `ToolCallApprovalRequested`, `ToolCallRejected` — paired with
  `ToolCallApproved` / `ToolCallCompleted`, both of which carry `at`.
- `ToolInvoked`, `ToolReturned` — fine-grained boundaries within a step;
  the enclosing `StepStarted` / `StepFinished` bracket the wall-clock
  window.

Adding `at` to any of these later is non-breaking (pure field addition).
Removing `at` from an existing stamped variant is breaking and requires
a wire-shape golden update.

## 2. Tag discriminator

- **Attribute**: `#[serde(tag = "type")]`.
- **Rename**: `rename_all = "snake_case"` for variants (unless the type
  is `Copy` and maps one-to-one onto a lowercase TS string union, in
  which case `rename_all = "lowercase"` is allowed).

All internally-tagged enums that cross the IPC boundary share the tag
name `"type"`. As of F-380 this applies to:

- `Event` (`forge_core::event::Event`) — already `tag = "type"` since
  Phase 0.
- `ServerState` (`forge_core::mcp_state::ServerState`) — migrated from
  `tag = "state"` in F-380.

### Pinned exceptions

| Type           | Current tag | Reason                                                  |
|----------------|-------------|---------------------------------------------------------|
| `StepOutcome`  | `status`    | AgentMonitor webview's `outcomeOf` helper in            |
|                |             | `web/packages/app/src/routes/AgentMonitor.tsx` reads    |
|                |             | `.status` directly. Rename requires a synchronized      |
|                |             | webview change and is deferred to a follow-up ticket.   |

Internal enums that do NOT cross the IPC boundary (e.g. `forge_lsp`'s
`Checksum`, internal shell command routing) are out of scope for this
convention. A type only enters the convention when its wire shape is
observed by a TS adapter, a ts-rs binding, or a cross-process JSON
payload.

## 3. Type mirroring (`ts-rs`)

Types that carry non-primitive fields across the webview boundary derive
`TS` and export to `web/packages/ipc/src/generated/`. `chrono::DateTime<Utc>`
is not known to ts-rs by default; annotate it with
`#[ts(type = "string")]` so the generated TS carries `at: string`.

## 4. Wire-shape tests

Every `Event` variant MUST have a golden-JSON pin in
`crates/forge-core/tests/event_wire_shape.rs`. The exhaustive `match` in
`variant_label` at the bottom of that file is the compile-time guard —
adding a new variant without a matching arm is a compile error.

F-380's cross-cutting rules are further locked by
`crates/forge-core/tests/event_conventions.rs`. Any change to rule #1,
#2, or the `McpStateEvent.at` wire shape must touch that file.
