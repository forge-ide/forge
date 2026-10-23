//! Event envelope, kinds, bodies, and subscription topics.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::ErrorData;
use crate::ids::{
    ApprovalId, CheckpointId, ProjectId, SessionId, SubscriptionId, ThreadId, TurnId,
};
use crate::model::{Action, EditKind, SessionState, Usage};
use crate::timestamp::Timestamp;

/// One entry in a session's event log.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    /// Per-session, gap-free.
    pub seq: u64,
    pub session_id: SessionId,
    pub at: Timestamp,
    /// Links a decision, hook run, or result to the tool call it belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub parent: Option<u64>,
    #[serde(flatten)]
    #[ts(flatten)]
    pub payload: EventPayload,
}

/// An event's `kind` and `body`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", content = "body")]
pub enum EventPayload {
    #[serde(rename = "session.state")]
    SessionState(SessionStateBody),
    #[serde(rename = "turn.started")]
    TurnStarted(TurnStartedBody),
    #[serde(rename = "turn.ended")]
    TurnEnded(TurnEndedBody),
    #[serde(rename = "prompt")]
    Prompt(PromptBody),
    #[serde(rename = "agent.message")]
    AgentMessage(AgentMessageBody),
    #[serde(rename = "agent.plan")]
    AgentPlan(AgentPlanBody),
    #[serde(rename = "tool.call")]
    ToolCall(ToolCallBody),
    #[serde(rename = "tool.result")]
    ToolResult(ToolResultBody),
    #[serde(rename = "policy.decision")]
    PolicyDecision(PolicyDecisionBody),
    #[serde(rename = "hook.run")]
    HookRun(HookRunBody),
    #[serde(rename = "approval.requested")]
    ApprovalRequested(ApprovalRequestedBody),
    #[serde(rename = "approval.decided")]
    ApprovalDecided(ApprovalDecidedBody),
    #[serde(rename = "edit")]
    Edit(EditBody),
    #[serde(rename = "terminal.output")]
    TerminalOutput(TerminalOutputBody),
    #[serde(rename = "checkpoint")]
    Checkpoint(CheckpointBody),
    #[serde(rename = "review.thread")]
    ReviewThread(ReviewThreadBody),
    #[serde(rename = "usage")]
    Usage(Usage),
    #[serde(rename = "error")]
    Error(ErrorData),
}

impl EventPayload {
    pub fn kind(&self) -> EventKind {
        match self {
            EventPayload::SessionState(_) => EventKind::SessionState,
            EventPayload::TurnStarted(_) => EventKind::TurnStarted,
            EventPayload::TurnEnded(_) => EventKind::TurnEnded,
            EventPayload::Prompt(_) => EventKind::Prompt,
            EventPayload::AgentMessage(_) => EventKind::AgentMessage,
            EventPayload::AgentPlan(_) => EventKind::AgentPlan,
            EventPayload::ToolCall(_) => EventKind::ToolCall,
            EventPayload::ToolResult(_) => EventKind::ToolResult,
            EventPayload::PolicyDecision(_) => EventKind::PolicyDecision,
            EventPayload::HookRun(_) => EventKind::HookRun,
            EventPayload::ApprovalRequested(_) => EventKind::ApprovalRequested,
            EventPayload::ApprovalDecided(_) => EventKind::ApprovalDecided,
            EventPayload::Edit(_) => EventKind::Edit,
            EventPayload::TerminalOutput(_) => EventKind::TerminalOutput,
            EventPayload::Checkpoint(_) => EventKind::Checkpoint,
            EventPayload::ReviewThread(_) => EventKind::ReviewThread,
            EventPayload::Usage(_) => EventKind::Usage,
            EventPayload::Error(_) => EventKind::Error,
        }
    }
}

/// Event kinds without bodies, for filters such as `events.query { kinds }`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub enum EventKind {
    #[serde(rename = "session.state")]
    SessionState,
    #[serde(rename = "turn.started")]
    TurnStarted,
    #[serde(rename = "turn.ended")]
    TurnEnded,
    #[serde(rename = "prompt")]
    Prompt,
    #[serde(rename = "agent.message")]
    AgentMessage,
    #[serde(rename = "agent.plan")]
    AgentPlan,
    #[serde(rename = "tool.call")]
    ToolCall,
    #[serde(rename = "tool.result")]
    ToolResult,
    #[serde(rename = "policy.decision")]
    PolicyDecision,
    #[serde(rename = "hook.run")]
    HookRun,
    #[serde(rename = "approval.requested")]
    ApprovalRequested,
    #[serde(rename = "approval.decided")]
    ApprovalDecided,
    #[serde(rename = "edit")]
    Edit,
    #[serde(rename = "terminal.output")]
    TerminalOutput,
    #[serde(rename = "checkpoint")]
    Checkpoint,
    #[serde(rename = "review.thread")]
    ReviewThread,
    #[serde(rename = "usage")]
    Usage,
    #[serde(rename = "error")]
    Error,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SessionStateBody {
    pub from: SessionState,
    pub to: SessionState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reason: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum TurnSource {
    Human,
    Review,
    Hook,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TurnStartedBody {
    pub turn_id: TurnId,
    pub source: TurnSource,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TurnEndedBody {
    pub turn_id: TurnId,
    pub stop_reason: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PromptBody {
    pub turn_id: TurnId,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentMessageBody {
    pub turn_id: TurnId,
    pub message_id: String,
    pub delta: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PlanEntry {
    pub content: String,
    pub status: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentPlanBody {
    pub entries: Vec<PlanEntry>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallBody {
    pub tool_call_id: String,
    pub action: Action,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ToolResultBody {
    pub tool_call_id: String,
    pub status: String,
    /// Truncated and redacted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub output: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    Allow,
    Deny,
    Ask,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PolicyDecisionBody {
    pub tool_call_id: String,
    pub verdict: Verdict,
    pub reason: String,
    pub source: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HookRunBody {
    pub hook: String,
    pub event: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub verdict: Option<Verdict>,
    pub duration_ms: u64,
    pub exit_code: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequestedBody {
    pub approval_id: ApprovalId,
    pub action: Action,
    pub reason: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalDecision {
    Allow,
    Deny,
}

/// How long an approval decision is remembered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum Remember {
    Session,
    Project,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalDecidedBody {
    pub approval_id: ApprovalId,
    pub decision: ApprovalDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub remember: Option<Remember>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EditBody {
    pub path: String,
    pub kind: EditKind,
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TerminalOutputBody {
    pub terminal_id: String,
    pub chunk: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointBody {
    pub checkpoint_id: CheckpointId,
    pub turn_id: TurnId,
    #[serde(rename = "ref")]
    #[ts(rename = "ref")]
    pub git_ref: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum ThreadChange {
    Created,
    Reply,
    Resolved,
    Outdated,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ReviewThreadBody {
    pub thread_id: ThreadId,
    pub change: ThreadChange,
}

/// A subscription topic: `board`, `approvals`, `session/<id>`, or `project/<id>`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(try_from = "String", into = "String")]
#[ts(type = "\"board\" | \"approvals\" | `session/${string}` | `project/${string}`")]
pub enum Topic {
    Board,
    Approvals,
    Session(SessionId),
    Project(ProjectId),
}

/// A topic string that doesn't name a known topic.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown topic {0:?}: expected board, approvals, session/<id>, or project/<id>")]
pub struct InvalidTopic(pub String);

impl FromStr for Topic {
    type Err = InvalidTopic;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let invalid = || InvalidTopic(s.to_owned());
        match s {
            "board" => Ok(Topic::Board),
            "approvals" => Ok(Topic::Approvals),
            _ => match s.split_once('/') {
                Some(("session", id)) => id.parse().map(Topic::Session).map_err(|_| invalid()),
                Some(("project", id)) => id.parse().map(Topic::Project).map_err(|_| invalid()),
                _ => Err(invalid()),
            },
        }
    }
}

impl TryFrom<String> for Topic {
    type Error = InvalidTopic;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<Topic> for String {
    fn from(t: Topic) -> String {
        t.to_string()
    }
}

impl fmt::Display for Topic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Topic::Board => f.write_str("board"),
            Topic::Approvals => f.write_str("approvals"),
            Topic::Session(id) => write!(f, "session/{id}"),
            Topic::Project(id) => write!(f, "project/{id}"),
        }
    }
}

/// Params of the `event` notification the daemon pushes to subscribers.
///
/// Each event reaches a subscription at most once, even when several of its topics match, so
/// `seq` strictly increases within a subscription.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EventNotification {
    pub subscription_id: SubscriptionId,
    /// The first of the event's topics that the subscription includes.
    pub topic: Topic,
    /// The daemon-wide stream cursor: increases across every session and topic and survives
    /// restarts. `events.subscribe { afterSeq }` resumes after it. Not the event's own `seq`,
    /// which counts within its session.
    pub seq: u64,
    pub event: Event,
}

/// Method name of the event notification.
pub const EVENT_NOTIFICATION: &str = "event";

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn event_envelope_matches_spec() {
        let session = SessionId::new();
        let event = Event {
            seq: 3,
            session_id: session.clone(),
            at: "2026-10-23T12:00:00Z".parse().unwrap(),
            parent: None,
            payload: EventPayload::SessionState(SessionStateBody {
                from: SessionState::Ready,
                to: SessionState::Working,
                reason: None,
            }),
        };
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(
            value,
            json!({
                "seq": 3,
                "sessionId": session.as_str(),
                "at": "2026-10-23T12:00:00Z",
                "kind": "session.state",
                "body": { "from": "ready", "to": "working" }
            })
        );
        assert_eq!(serde_json::from_value::<Event>(value).unwrap(), event);
        assert_eq!(event.payload.kind(), EventKind::SessionState);
    }

    #[test]
    fn checkpoint_body_uses_ref() {
        let body = CheckpointBody {
            checkpoint_id: CheckpointId::new(),
            turn_id: TurnId::new(),
            git_ref: "refs/forge/x/cp/1".into(),
        };
        assert_eq!(
            serde_json::to_value(&body).unwrap()["ref"],
            "refs/forge/x/cp/1"
        );
    }

    #[test]
    fn topics_parse_and_print() {
        let session = SessionId::new();
        for s in [
            "board".to_owned(),
            "approvals".to_owned(),
            format!("session/{session}"),
        ] {
            assert_eq!(s.parse::<Topic>().unwrap().to_string(), s);
        }
        assert!(
            "session/prj_01J00000000000000000000000"
                .parse::<Topic>()
                .is_err()
        );
        assert!("boards".parse::<Topic>().is_err());
        assert!(serde_json::from_value::<Topic>(json!("nope")).is_err());
    }
}
