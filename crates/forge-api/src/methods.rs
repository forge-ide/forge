//! The v0.1 daemon method set: one marker type per method, with its params and result.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::events::{ApprovalDecision, Event, EventKind, Remember, Topic};
use crate::ids::{
    ApprovalId, CheckpointId, HunkId, ProjectId, SessionId, SubscriptionId, ThreadId, TurnId,
};
use crate::model::{
    AgentProfile, Approval, Changeset, Hunk, LineRange, Mode, Platform, Project, SandboxTier,
    Session, SessionState, SignInMethod, Thread, ThreadStatus,
};

/// A daemon method: its wire name and the types of its params and result.
pub trait Method {
    const NAME: &'static str;
    type Params: Serialize + DeserializeOwned + Send + Sync;
    type Result: Serialize + DeserializeOwned + Send + Sync;
}

/// Params or result written as `{}` on the wire.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(type = "Record<string, never>")]
pub struct Empty {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HelloParams {
    /// Free-form client name, for logs: `forge-cli`, `forge-desktop`.
    pub client: String,
    /// The API version the client was built against.
    pub api_version: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HelloResult {
    pub daemon_version: String,
    pub api_version: String,
    pub platform: Platform,
    pub sandbox_tiers: Vec<SandboxTier>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectOpenParams {
    /// Absolute path to the project root. Clients resolve relative paths first.
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectGetParams {
    pub project_id: ProjectId,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentListParams {
    /// List this project's profiles: the built-in ones and its `.forge/agents/` files. Without
    /// it, only the built-in profiles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub project_id: Option<ProjectId>,
}

/// A registry agent, for signing in. Only registry agents sign in: signing in runs the agent
/// outside the sandbox, so never a command from a project's profile file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentRefParams {
    /// Registry key: `claude-code`, `codex`, `gemini`, or another adapter.
    pub agent: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentSignInParams {
    pub agent: String,
    /// One of the agent's non-terminal sign-in methods, from `agent.signInMethods`.
    pub method_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SessionCreateParams {
    pub project_id: ProjectId,
    pub agent_profile: String,
    pub title: String,
    pub prompt: String,
    pub mode: Mode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub sandbox: Option<SandboxTier>,
    /// Full branch name; derived from `title` and the configured format when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub branch: Option<String>,
    /// What to do when the branch or worktree name is already taken. Without it, a collision
    /// fails with `Conflict` so the client can ask the user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub on_collision: Option<CollisionResolution>,
}

/// The user's answer to a branch or worktree name collision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum CollisionResolution {
    /// Check out the existing branch in a new worktree.
    ReuseBranch,
    /// Use the existing worktree, which no live session owns.
    AdoptWorktree,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct BranchPreviewParams {
    pub project_id: ProjectId,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub branch: Option<String>,
}

/// The branch a new session would use, and what already holds that name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct BranchPreview {
    pub branch: String,
    pub collisions: Vec<BranchCollision>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct BranchCollision {
    /// Repo name from `forge.toml`.
    pub repo: String,
    pub kind: CollisionKind,
    /// The live session that owns the worktree, for `owned-by-live-session`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub session_id: Option<SessionId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum CollisionKind {
    BranchExists,
    WorktreeExists,
    OwnedByLiveSession,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SessionPromptParams {
    pub session_id: SessionId,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TurnResult {
    pub turn_id: TurnId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SessionIdParams {
    pub session_id: SessionId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SessionSetModeParams {
    pub session_id: SessionId,
    pub mode: Mode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub sandbox: Option<SandboxTier>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SessionRewindParams {
    pub session_id: SessionId,
    pub checkpoint_id: CheckpointId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SessionListParams {
    pub project_id: ProjectId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub states: Option<Vec<SessionState>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum SessionOutcome {
    Shipped,
    Abandoned,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SessionCloseParams {
    pub session_id: SessionId,
    pub outcome: SessionOutcome,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalListParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub project_id: Option<ProjectId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalDecideParams {
    pub approval_id: ApprovalId,
    pub decision: ApprovalDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub remember: Option<Remember>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub note: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ReviewChangesetParams {
    pub session_id: SessionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub from_checkpoint: Option<CheckpointId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFileDiffParams {
    pub session_id: SessionId,
    /// The repo the file is in; may be left out when the session has only one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub repo: Option<String>,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub from_checkpoint: Option<CheckpointId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FileDiffResult {
    pub hunks: Vec<Hunk>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCommentParams {
    pub session_id: SessionId,
    /// The repo the file is in; may be left out when the session has only one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub repo: Option<String>,
    pub path: String,
    pub range: LineRange,
    pub body: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSetViewedParams {
    pub session_id: SessionId,
    /// The repo the file is in; may be left out when the session has only one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub repo: Option<String>,
    pub path: String,
    pub viewed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ReviewReplyParams {
    pub thread_id: ThreadId,
    pub body: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ThreadIdParams {
    pub thread_id: ThreadId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ReviewThreadsParams {
    pub session_id: SessionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub status: Option<ThreadStatus>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRevertHunkParams {
    pub session_id: SessionId,
    /// Which repo, when the session spans more than one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub repo: Option<String>,
    /// Spelled as `review.changeset` gives it.
    pub path: String,
    pub hunk_id: HunkId,
    /// The `fromCheckpoint` of the diff the hunk was shown in, such as Since last review's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub from_checkpoint: Option<CheckpointId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum ShipStrategy {
    PushBranch,
    MergeLocal,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ReviewShipParams {
    pub session_id: SessionId,
    /// The checkpoint the human reviewed: the changeset's `head`. Ship refuses if it is no
    /// longer the newest, or the worktree or branch changed since.
    pub checkpoint_id: CheckpointId,
    pub strategy: ShipStrategy,
    /// `push-branch` only: the remote branch to push to; the session's branch by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub branch: Option<String>,
    /// Ship even though some files were withheld from review (too large to show).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub include_withheld: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ShipResult {
    #[serde(rename = "ref")]
    #[ts(rename = "ref")]
    pub git_ref: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FileReadParams {
    pub project_id: ProjectId,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub session_id: Option<SessionId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FileReadResult {
    pub content: String,
    pub hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FileWriteParams {
    pub project_id: ProjectId,
    pub path: String,
    pub content: String,
    pub base_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub session_id: Option<SessionId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FileWriteResult {
    pub hash: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LspRequestParams {
    pub project_id: ProjectId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub session_id: Option<SessionId>,
    pub method: String,
    #[ts(type = "unknown")]
    pub params: serde_json::Value,
}

/// A raw LSP result, passed through unchanged.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(transparent)]
#[ts(type = "unknown")]
pub struct LspResult(pub serde_json::Value);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EventsSubscribeParams {
    pub topics: Vec<Topic>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub after_seq: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SubscribeResult {
    pub subscription_id: SubscriptionId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EventsUnsubscribeParams {
    pub subscription_id: SubscriptionId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EventsQueryParams {
    pub session_id: SessionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub kinds: Option<Vec<EventKind>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub text: Option<String>,
    /// Only events with a lower `seq`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub before: Option<u64>,
    pub limit: u32,
}

/// Declares marker types for methods and a table of them for code generation.
macro_rules! methods {
    ($($(#[$doc:meta])* $marker:ident = $name:literal : $params:ty => $result:ty;)*) => {
        $(
            $(#[$doc])*
            #[derive(Clone, Copy, Debug)]
            pub struct $marker;

            impl Method for $marker {
                const NAME: &'static str = $name;
                type Params = $params;
                type Result = $result;
            }
        )*

        /// Every method name, in spec order.
        pub const METHOD_NAMES: &[&str] = &[$($name),*];

        /// `"name": { params: P; result: R }` entries for the TypeScript method map.
        pub(crate) fn method_map_entries(cfg: &ts_rs::Config) -> Vec<String> {
            vec![$(format!(
                "  {:?}: {{ params: {}; result: {} }};",
                $name,
                <$params as TS>::name(cfg),
                <$result as TS>::name(cfg),
            )),*]
        }
    };
}

methods! {
    ServerHello = "server.hello": HelloParams => HelloResult;
    ProjectOpen = "project.open": ProjectOpenParams => Project;
    ProjectList = "project.list": Empty => Vec<Project>;
    ProjectGet = "project.get": ProjectGetParams => Project;
    AgentList = "agent.list": AgentListParams => Vec<AgentProfile>;
    AgentSignInMethods = "agent.signInMethods": AgentRefParams => Vec<SignInMethod>;
    AgentSignIn = "agent.signIn": AgentSignInParams => Empty;
    SessionCreate = "session.create": SessionCreateParams => Session;
    SessionPrompt = "session.prompt": SessionPromptParams => TurnResult;
    SessionCancel = "session.cancel": SessionIdParams => Empty;
    SessionSetMode = "session.setMode": SessionSetModeParams => Session;
    SessionRewind = "session.rewind": SessionRewindParams => Session;
    SessionGet = "session.get": SessionIdParams => Session;
    SessionList = "session.list": SessionListParams => Vec<Session>;
    SessionClose = "session.close": SessionCloseParams => Empty;
    SessionBranchPreview = "session.branchPreview": BranchPreviewParams => BranchPreview;
    ApprovalList = "approval.list": ApprovalListParams => Vec<Approval>;
    ApprovalDecide = "approval.decide": ApprovalDecideParams => Empty;
    ReviewChangeset = "review.changeset": ReviewChangesetParams => Changeset;
    ReviewFileDiff = "review.fileDiff": ReviewFileDiffParams => FileDiffResult;
    ReviewComment = "review.comment": ReviewCommentParams => Thread;
    ReviewReply = "review.reply": ReviewReplyParams => Thread;
    ReviewResolve = "review.resolve": ThreadIdParams => Thread;
    ReviewThreads = "review.threads": ReviewThreadsParams => Vec<Thread>;
    ReviewSetViewed = "review.setViewed": ReviewSetViewedParams => Empty;
    ReviewRequestChanges = "review.requestChanges": SessionIdParams => TurnResult;
    ReviewRevertHunk = "review.revertHunk": ReviewRevertHunkParams => Empty;
    ReviewShip = "review.ship": ReviewShipParams => ShipResult;
    FileRead = "file.read": FileReadParams => FileReadResult;
    FileWrite = "file.write": FileWriteParams => FileWriteResult;
    LspRequest = "lsp.request": LspRequestParams => LspResult;
    EventsSubscribe = "events.subscribe": EventsSubscribeParams => SubscribeResult;
    EventsUnsubscribe = "events.unsubscribe": EventsUnsubscribeParams => Empty;
    EventsQuery = "events.query": EventsQueryParams => Vec<Event>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn method_names_are_unique() {
        let mut names = METHOD_NAMES.to_vec();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), METHOD_NAMES.len());
        assert_eq!(METHOD_NAMES.len(), 34);
    }

    #[test]
    fn empty_is_an_empty_object() {
        assert_eq!(serde_json::to_value(Empty {}).unwrap(), json!({}));
        assert_eq!(
            serde_json::from_value::<Empty>(json!({})).unwrap(),
            Empty {}
        );
    }

    #[test]
    fn subscribe_params_parse_topics() {
        let params: EventsSubscribeParams =
            serde_json::from_value(json!({ "topics": ["board"], "afterSeq": 7 })).unwrap();
        assert_eq!(params.topics, vec![Topic::Board]);
        assert_eq!(params.after_seq, Some(7));
    }
}
