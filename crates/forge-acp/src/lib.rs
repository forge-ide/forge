//! ACP client.
//!
//! [`AcpClient`] starts an agent process and speaks ACP to it over stdio. It serves the agent's
//! file reads and writes inside the session's [`Workspace`], runs its terminals, asks a
//! [`Gate`] before each permission request, write, and terminal, and turns what happens into
//! session events for an [`EventSink`].

pub mod adapters;
pub mod client;
pub mod logins;
pub mod permission;
pub mod shim;
pub mod terminal;
pub mod workspace;

pub use client::{
    AcpClient, AcpError, AgentCommand, AgentExit, AllowAll, DenyAll, EventSink, Gate, GateKind,
    GateRequest, GateVerdict, McpEndpoint, SessionSetup, Unmapped,
};
pub use terminal::Pipe;
pub use workspace::{PathError, Resolved, Root, Workspace};
