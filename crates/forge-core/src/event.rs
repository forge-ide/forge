use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;

use crate::ids::{AgentId, AgentInstanceId, MessageId, ProviderId, StepId, ToolCallId};
use crate::mcp_state::McpStateEvent;
use crate::types::{
    ApprovalScope, CompactTrigger, RosterScope, SessionPersistence, StepKind, StepOutcome,
    TokenUsage,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ContextRef {
    File(PathBuf),
    Url(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovalPreview {
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ApprovalSource {
    User,
    Auto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EndReason {
    UserExit,
    Error(String),
    Completed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    SessionStarted {
        at: DateTime<Utc>,
        workspace: PathBuf,
        agent: Option<AgentId>,
        persistence: SessionPersistence,
    },
    UserMessage {
        id: MessageId,
        at: DateTime<Utc>,
        // F-112: `Arc<str>` for hot per-token IPC fields — cheap clone on fanout,
        // one allocation at the upstream producer instead of per-subscriber copies.
        // Serializes identically to `String` (plain JSON string), so the wire
        // shape pinned by `event_wire_shape.rs` is unchanged.
        text: Arc<str>,
        context: Vec<ContextRef>,
        branch_parent: Option<MessageId>,
    },
    AssistantMessage {
        id: MessageId,
        provider: ProviderId,
        model: String,
        at: DateTime<Utc>,
        stream_finalised: bool,
        text: Arc<str>,
        branch_parent: Option<MessageId>,
        branch_variant_index: u32,
    },
    AssistantDelta {
        id: MessageId,
        at: DateTime<Utc>,
        delta: Arc<str>,
    },
    BranchSelected {
        parent: MessageId,
        selected: MessageId,
    },
    /// F-145: marks a branch variant as logically deleted. The event log is
    /// append-only (§15.1) — the underlying `AssistantMessage` is not removed.
    /// Replay consumers hide any assistant events whose `(branch_parent, index)`
    /// (or `(id, 0)` for roots) matches a `BranchDeleted` marker. Used by the
    /// branch metadata popover's Delete action.
    ///
    /// Deleting `variant_index == 0` is rejected server-side — the root is the
    /// original message and removing it would orphan every sibling.
    BranchDeleted {
        parent: MessageId,
        variant_index: u32,
    },
    /// F-143: emitted after a successful re-run (Replace variant) to mark
    /// `old_id`'s assistant message as logically superseded by `new_id`.
    ///
    /// The event log itself is append-only — the superseded events remain on
    /// disk. Replay filters (see `forge_core::apply_superseded`) consult
    /// these markers so late-joining subscribers see a coherent transcript
    /// where the regenerated message takes the original's place.
    MessageSuperseded {
        old_id: MessageId,
        new_id: MessageId,
    },
    ToolCallStarted {
        id: ToolCallId,
        msg: MessageId,
        tool: String,
        args: Value,
        at: DateTime<Utc>,
        parallel_group: Option<u32>,
    },
    ToolCallApprovalRequested {
        id: ToolCallId,
        preview: ApprovalPreview,
    },
    ToolCallApproved {
        id: ToolCallId,
        by: ApprovalSource,
        scope: ApprovalScope,
        at: DateTime<Utc>,
    },
    ToolCallRejected {
        id: ToolCallId,
        reason: Option<String>,
    },
    ToolCallCompleted {
        id: ToolCallId,
        result: Value,
        duration_ms: u64,
        at: DateTime<Utc>,
    },
    SubAgentSpawned {
        parent: AgentInstanceId,
        child: AgentInstanceId,
        from_msg: MessageId,
    },
    BackgroundAgentStarted {
        id: AgentInstanceId,
        agent: AgentId,
        at: DateTime<Utc>,
    },
    BackgroundAgentCompleted {
        id: AgentInstanceId,
        at: DateTime<Utc>,
    },
    UsageTick {
        provider: ProviderId,
        model: String,
        tokens_in: u64,
        tokens_out: u64,
        cost_usd: f64,
        scope: RosterScope,
    },
    ContextCompacted {
        at: DateTime<Utc>,
        summarized_turns: u32,
        summary_msg_id: MessageId,
        trigger: CompactTrigger,
    },
    SessionEnded {
        at: DateTime<Utc>,
        reason: EndReason,
        archived: bool,
    },
    /// F-139: fine-grained step trace — opens a step within a turn.
    ///
    /// Emitted by the session turn loop before any `AssistantMessage`,
    /// `AssistantDelta`, or `ToolCall*` event that logically belongs to
    /// the step. Every `StepStarted` is terminated by exactly one
    /// `StepFinished` carrying the same `step_id` (or the session ends
    /// abnormally and replay consumers treat unterminated steps as
    /// failed).
    ///
    /// `instance_id` is optional because the session-level turn loop
    /// does not run inside an `AgentInstance` today; F-140 wires the
    /// `AgentMonitor` through `run_turn` and will populate the field.
    /// Until then top-level turns emit `instance_id: None`.
    ///
    /// Ordering invariant (see `forge-session::orchestrator`):
    /// `StepStarted` < any `AssistantMessage` / `AssistantDelta` /
    /// `ToolCallStarted` / `ToolCallApproved` / `ToolCallCompleted` /
    /// `ToolInvoked` / `ToolReturned` with the same `step_id` <
    /// `StepFinished`.
    StepStarted {
        step_id: StepId,
        instance_id: Option<AgentInstanceId>,
        kind: StepKind,
        started_at: DateTime<Utc>,
    },
    /// F-139: fine-grained step trace — closes a step.
    ///
    /// `duration_ms` is wall-clock elapsed between the matching
    /// `StepStarted.started_at` and this event's emission moment.
    /// `token_usage` is `Some` only when the provider reported per-step
    /// usage (today always `None` — mock provider doesn't emit usage;
    /// F-155 populates it).
    ///
    /// Terminates the step; no further events may reference the same
    /// `step_id`.
    StepFinished {
        step_id: StepId,
        outcome: StepOutcome,
        duration_ms: u64,
        token_usage: Option<TokenUsage>,
    },
    /// F-139: tool invocation observed inside a tool-step.
    ///
    /// Emitted at the boundary between approval and execution — after
    /// `ToolCallApproved` (or the auto-approve emission) and before
    /// `tool.invoke`. `args_digest` is a short SHA-256 hex prefix of the
    /// serialized args JSON; downstream UIs correlate it with
    /// `ToolCallStarted.args` without re-hashing the full payload.
    /// `tool_id` is the same string key registered on the dispatcher
    /// (e.g. `"fs.read"`).
    ToolInvoked {
        step_id: StepId,
        tool_call_id: ToolCallId,
        tool_id: String,
        args_digest: String,
    },
    /// F-139: tool invocation returned.
    ///
    /// Emitted immediately after `tool.invoke` completes and before
    /// `ToolCallCompleted`. `bytes_out` is the length of the serialized
    /// result JSON in UTF-8 bytes; `ok` is `true` when the result did
    /// not serialize an `error` field at the top level.
    ToolReturned {
        step_id: StepId,
        tool_call_id: ToolCallId,
        ok: bool,
        bytes_out: u64,
    },
    /// F-155: MCP server lifecycle transition.
    ///
    /// Emitted on the session event log by the daemon's state-stream
    /// forwarder whenever its single authoritative `McpManager` publishes
    /// a `McpStateEvent`. Subscribers (the shell's session event forwarder
    /// and any late-joining replay consumer) observe the exact set of
    /// `Starting / Healthy / Degraded / Failed / Disabled` transitions
    /// that drove the manager. The event is informational — it is not a
    /// command and the log is append-only, so `apply_superseded` leaves
    /// it alone.
    McpState(McpStateEvent),
}
