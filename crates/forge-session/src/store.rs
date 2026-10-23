//! Where session metadata is persisted. The daemon implements this over SQLite.

use std::path::{Path, PathBuf};

use forge_api::model::{Session, SessionState};
use forge_api::{ProjectId, SessionId, Timestamp};

#[derive(Debug, Clone, thiserror::Error)]
#[error("session store: {0}")]
pub struct StoreError(pub String);

/// A session's worktree in one repo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorktreeRecord {
    pub repo: String,
    pub path: PathBuf,
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
}
