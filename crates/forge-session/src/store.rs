//! Where session metadata is persisted. The daemon implements this over SQLite.

use std::path::{Path, PathBuf};

use forge_api::events::Remember;
use forge_api::model::{Approval, Mode, Session, SessionState};
use forge_api::{ApprovalId, CheckpointId, ProjectId, SessionId, Timestamp, TurnId};

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

/// A checkpoint: one commit per repo, on its shadow ref.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckpointRecord {
    pub id: CheckpointId,
    pub session: SessionId,
    /// Counts up from 1 per session, discarded checkpoints included.
    pub n: u32,
    /// The turn it was taken after, if any.
    pub turn: Option<TurnId>,
    pub created_at: Timestamp,
    /// Each repo's commit, by repo name.
    pub commits: Vec<(String, String)>,
    /// The checkpoint a rewind went back to, once one discarded this one.
    pub discarded_by: Option<CheckpointId>,
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

    fn insert_checkpoint(&self, checkpoint: &CheckpointRecord) -> Result<(), StoreError>;
    /// Every checkpoint of a session, discarded ones included, in order.
    fn checkpoints(&self, session: &SessionId) -> Result<Vec<CheckpointRecord>, StoreError>;
    /// Mark the session's checkpoints after the `n`th that aren't discarded yet as discarded
    /// by a rewind to `by`.
    fn discard_checkpoints(
        &self,
        session: &SessionId,
        n: u32,
        by: &CheckpointId,
    ) -> Result<(), StoreError>;

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
