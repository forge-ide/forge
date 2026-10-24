//! Where session metadata is persisted. The daemon implements this over SQLite.

use std::path::{Path, PathBuf};

use forge_api::events::Remember;
use forge_api::model::{Approval, Mode, Session, SessionState};
use forge_api::{ApprovalId, ProjectId, SessionId, Timestamp};

#[derive(Debug, Clone, thiserror::Error)]
#[error("session store: {0}")]
pub struct StoreError(pub String);

/// A session's worktree in one repo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorktreeRecord {
    pub repo: String,
    pub path: PathBuf,
}

/// How an approval ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApprovalOutcome {
    Allow,
    Deny,
    /// The turn was cancelled or the daemon restarted before anyone decided.
    Cancelled,
}

/// An approval and whether it has ended.
#[derive(Clone, Debug, PartialEq)]
pub struct ApprovalRecord {
    pub approval: Approval,
    pub decided: bool,
}

/// Session persistence. Calls are blocking; the manager runs them on the blocking pool.
pub trait SessionStore: Send + Sync + 'static {
    fn insert(&self, session: &Session, worktrees: &[WorktreeRecord]) -> Result<(), StoreError>;
    fn set_state(
        &self,
        id: &SessionId,
        state: SessionState,
        at: Timestamp,
    ) -> Result<(), StoreError>;
    fn get(&self, id: &SessionId) -> Result<Option<Session>, StoreError>;
    /// A project's sessions, newest first, optionally only those in `states`.
    fn list(
        &self,
        project: &ProjectId,
        states: Option<&[SessionState]>,
    ) -> Result<Vec<Session>, StoreError>;
    /// Every session in any of `states`, across projects.
    fn in_states(&self, states: &[SessionState]) -> Result<Vec<Session>, StoreError>;
    fn worktrees(&self, id: &SessionId) -> Result<Vec<WorktreeRecord>, StoreError>;
    /// The live session (not Done or Failed) whose worktree is at `path`, if any.
    fn live_owner(&self, path: &Path) -> Result<Option<SessionId>, StoreError>;
    fn set_mode(&self, id: &SessionId, mode: Mode, at: Timestamp) -> Result<(), StoreError>;

    fn insert_approval(&self, approval: &Approval) -> Result<(), StoreError>;
    fn approval(&self, id: &ApprovalId) -> Result<Option<ApprovalRecord>, StoreError>;
    /// Record how a pending approval ended; `false` if it had already ended.
    fn end_approval(
        &self,
        id: &ApprovalId,
        outcome: ApprovalOutcome,
        remember: Option<Remember>,
        at: Timestamp,
    ) -> Result<bool, StoreError>;
    /// Pending approvals, oldest first, optionally only one project's.
    fn pending_approvals(&self, project: Option<&ProjectId>) -> Result<Vec<Approval>, StoreError>;
    /// Cancel every pending approval and return them, oldest first.
    fn cancel_pending_approvals(&self, at: Timestamp) -> Result<Vec<Approval>, StoreError>;
}
