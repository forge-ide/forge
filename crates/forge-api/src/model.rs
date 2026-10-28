//! Core types shared by methods and events.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::{
    ApprovalId, CheckpointId, CommentId, HunkId, ProjectId, SessionId, ThreadId, TurnId,
};
use crate::timestamp::Timestamp;

/// How much the agent may do without asking.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    Ask,
    AcceptEdits,
    Auto,
    FullAccess,
}

/// Isolation tier for a session's agent process.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum SandboxTier {
    Light,
    Standard,
    Container,
    Unsandboxed,
}

/// Session lifecycle state (component-architecture.md, Session manager).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum SessionState {
    Provisioning,
    Ready,
    Working,
    Waiting,
    Paused,
    Review,
    Done,
    Failed,
}

/// Operating system the daemon runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum Platform {
    Linux,
    Macos,
}

impl Platform {
    /// The platform this binary was built for.
    pub const fn current() -> Self {
        #[cfg(target_os = "macos")]
        {
            Platform::Macos
        }
        #[cfg(not(target_os = "macos"))]
        {
            Platform::Linux
        }
    }
}

/// An opened project. `root` is the one absolute path that crosses the API.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub root: String,
    pub repos: Vec<Repo>,
}

/// A repo within a project; `path` is relative to the project root.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Repo {
    pub name: String,
    pub path: String,
    pub base_branch: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentProfile {
    pub id: String,
    /// Registry key: `claude-code`, `codex`, `gemini`, or another adapter.
    pub agent: String,
    pub installed: bool,
    /// Every secret the profile names is in the keyring, or for a registry agent with none
    /// of them there, its own subscription sign-in is present and unexpired.
    pub authenticated: bool,
    /// The installed version, when Forge can tell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub version: Option<String>,
    /// The agent's display name, for a registry agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
    /// The version Forge is tested with, for a registry agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub supported_version: Option<String>,
    /// The command that installs the supported version, when the agent isn't installed or
    /// is at another version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub install_command: Option<String>,
    /// How to sign in, when the agent isn't signed in: the command that stores each missing
    /// secret in the keyring, or the agent's own command that signs in with a subscription.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub sign_in: Option<String>,
    /// Why a session with this profile can't start, when Forge can tell before starting one
    /// (an invalid profile file, an unknown agent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub problem: Option<String>,
}

/// A way an agent offers to sign in, as it advertises it over ACP.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SignInMethod {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub description: Option<String>,
    /// For a method the agent runs in a terminal: the whole command, which the client runs
    /// interactively outside the sandbox, as the user would. Without it, `agent.signIn` runs
    /// the method, and the agent does the flow itself (a browser, say).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub terminal: Option<TerminalSignIn>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSignIn {
    pub argv: Vec<String>,
    /// Added to the client's own environment.
    pub env: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cost_usd: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: SessionId,
    pub project_id: ProjectId,
    pub title: String,
    pub agent_profile: String,
    pub state: SessionState,
    pub mode: Mode,
    pub sandbox: SandboxTier,
    pub branch: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub usage: Usage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum EditKind {
    Create,
    Modify,
    Delete,
}

/// Something an agent wants to do; every action gets exactly one policy decision.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum Action {
    Edit {
        path: String,
        kind: EditKind,
    },
    Exec {
        argv: Vec<String>,
        cwd: String,
    },
    Network {
        host: String,
        port: u16,
    },
    Mcp {
        server: String,
        tool: String,
        #[ts(type = "unknown")]
        args: serde_json::Value,
    },
    Git {
        op: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Approval {
    pub id: ApprovalId,
    pub session_id: SessionId,
    pub action: Action,
    pub reason: String,
    pub created_at: Timestamp,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Hunk {
    pub id: HunkId,
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub lines: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub turn_id: Option<TurnId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    /// The repo the file is in, by its name in `forge.toml`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub repo: Option<String>,
    pub path: String,
    pub status: FileStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub old_path: Option<String>,
    /// Each side's mode, in octal as git prints it (`100644`, `100755`, `120000` for a
    /// symlink, `160000` for a submodule), when the file is on that side.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub old_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub new_mode: Option<String>,
    pub additions: u32,
    pub deletions: u32,
    /// A side held a NUL byte or bytes that aren't UTF-8. Its hunks and counts are still
    /// there, with each such byte shown as U+FFFD; clients collapse it by default.
    #[serde(default)]
    pub binary: bool,
    /// A side is over 16 MiB or 1,000,000 lines: no line counts, and `review.fileDiff`
    /// refuses it. Clients show it as a warning, never as an unchanged-looking 0/0.
    #[serde(default)]
    pub too_large: bool,
    /// The human marked it viewed in this review round.
    #[serde(default)]
    pub viewed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Changeset {
    pub session_id: SessionId,
    pub base: String,
    pub head: String,
    pub files: Vec<FileChange>,
    /// The review round: how many times changes were requested, plus one.
    pub round: u32,
    /// The checkpoint the last request for changes reviewed, for "since last review".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub last_reviewed: Option<CheckpointId>,
}

/// Inclusive, 1-based line range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LineRange {
    pub start_line: u32,
    pub end_line: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum ThreadStatus {
    Open,
    Resolved,
    Outdated,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub id: ThreadId,
    pub session_id: SessionId,
    /// The repo the file is in, by its name in `forge.toml`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub repo: Option<String>,
    /// The file's path at `checkpoint_id`, as `review.changeset` gives paths.
    pub path: String,
    /// The lines it was written on, at `checkpoint_id`.
    pub range: LineRange,
    pub checkpoint_id: CheckpointId,
    /// Those lines as they were then.
    pub quote: Vec<String>,
    /// Where those lines are at the newest checkpoint; absent once the thread is outdated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub current: Option<LineRange>,
    pub status: ThreadStatus,
    pub comments: Vec<Comment>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum CommentAuthor {
    Human,
    Agent,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    pub id: CommentId,
    pub author: CommentAuthor,
    pub body: String,
    pub created_at: Timestamp,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn enums_use_spec_spellings() {
        assert_eq!(
            serde_json::to_value(Mode::AcceptEdits).unwrap(),
            json!("accept-edits")
        );
        assert_eq!(
            serde_json::to_value(Mode::FullAccess).unwrap(),
            json!("full-access")
        );
        assert_eq!(
            serde_json::to_value(SandboxTier::Unsandboxed).unwrap(),
            json!("unsandboxed")
        );
        assert_eq!(
            serde_json::to_value(SessionState::Provisioning).unwrap(),
            json!("provisioning")
        );
        assert_eq!(
            serde_json::to_value(Platform::Macos).unwrap(),
            json!("macos")
        );
    }

    #[test]
    fn action_is_tagged_by_type() {
        let action = Action::Exec {
            argv: vec!["cargo".into(), "test".into()],
            cwd: ".".into(),
        };
        assert_eq!(
            serde_json::to_value(&action).unwrap(),
            json!({ "type": "exec", "argv": ["cargo", "test"], "cwd": "." })
        );
        let edit: Action =
            serde_json::from_value(json!({ "type": "edit", "path": "a.rs", "kind": "create" }))
                .unwrap();
        assert_eq!(
            edit,
            Action::Edit {
                path: "a.rs".into(),
                kind: EditKind::Create
            }
        );
    }

    #[test]
    fn optional_fields_are_omitted() {
        let usage = Usage {
            input_tokens: 1,
            output_tokens: 2,
            cost_usd: None,
        };
        assert_eq!(
            serde_json::to_value(&usage).unwrap(),
            json!({ "inputTokens": 1, "outputTokens": 2 })
        );
    }
}
