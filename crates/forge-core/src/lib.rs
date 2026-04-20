pub mod approvals;
mod error;
mod event;
mod event_log;
pub mod ids;
pub mod meta;
mod transcript;
pub mod types;
pub mod workspace;
pub mod workspaces;

pub use approvals::{ApprovalConfig, ApprovalEntry};
pub use error::{ForgeError, Result};
pub use event::{ApprovalPreview, ApprovalSource, ContextRef, EndReason, Event};
pub use event_log::{read_since, EventLog, MAX_LINE_BYTES};
pub use ids::{
    AgentId, AgentInstanceId, MessageId, ProviderId, SessionId, TerminalId, ToolCallId, WorkspaceId,
};
pub use transcript::{apply_superseded, Transcript};
pub use types::{
    ApprovalLevel, ApprovalScope, CompactTrigger, RerunVariant, RosterScope, SessionPersistence,
    SessionState,
};
