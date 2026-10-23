//! Session lifecycle and supervision.
//!
//! [`SessionManager`] creates sessions (branch, worktrees, agent), runs their turns, and moves
//! them through the state machine in [`state`], persisting each change through a
//! [`SessionStore`] and recording it in the event log.

pub mod manager;
pub mod naming;
pub mod state;
pub mod store;

pub use manager::{AgentResolver, CreateRequest, SessionError, SessionManager};
pub use store::{SessionStore, StoreError, WorktreeRecord};
