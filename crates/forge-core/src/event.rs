use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;

use crate::ids::{AgentId, AgentInstanceId, MessageId, ProviderId, ToolCallId};
use crate::types::{ApprovalScope, CompactTrigger, RosterScope, SessionPersistence};

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
}
