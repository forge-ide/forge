//! ACP client.
//!
//! [`AcpClient`] starts an agent process and speaks ACP to it over stdio. It serves the agent's
//! file reads and writes inside the session's [`Workspace`], runs its terminals, passes
//! permission requests to a [`PermissionHandler`], and turns what happens into session events
//! for an [`EventSink`].

pub mod client;
pub mod terminal;
pub mod workspace;

pub use client::{
    AcpClient, AcpError, AgentCommand, AgentExit, DenyAll, EventSink, PermissionDecision,
    PermissionHandler, PermissionQuery,
};
pub use workspace::{PathError, Resolved, Root, Workspace};
