#![deny(rustdoc::broken_intra_doc_links, rustdoc::private_intra_doc_links)]
//! # forge-lsp
//!
//! Server-side LSP lifecycle: registry, download bootstrap, stdio process
//! supervision, and a byte-transparent [`MessageTransport`] bridge between
//! the parent webview and a spawned language server. F-123.
//!
//! ## Scope divergence from `docs/architecture/crate-architecture.md` §3.7
//!
//! The architecture doc describes `forge-lsp` as a *management* layer that
//! "doesn't proxy LSP messages" — the original plan had `monaco-languageclient`
//! in the webview reach servers directly. F-121 lifted Monaco into an iframe
//! whose parent webview owns the IPC bridge; this task (F-123) wires that
//! bridge to `forge-lsp`, so this crate now both manages and proxies.
//!
//! The proxy is a *byte-transparent relay*: this crate never parses LSP
//! frames. It shuttles opaque JSON values between the iframe (via Tauri IPC
//! plus postMessage) and the server's stdio. That keeps the sandbox story
//! honest: server output can't reach the filesystem, only the webview.
//!
//! F-148 is a separate doc-reconcile follow-up that will update §3.7 to
//! match the actual shape.
//!
//! ## Modules
//!
//! - [`registry`]: static [`Registry`] of bundled `ServerSpec`s (language id,
//!   binary name, download URL, checksum).
//! - [`bootstrap`]: [`Bootstrap::ensure`] downloads + checksum-verifies a
//!   server into `~/.cache/forge/lsp/<server_id>/`, honoring the sandbox.
//! - [`server`]: [`Server::start`] supervises a stdio child with
//!   restart-with-backoff (max 5 retries / 10 min), plus the
//!   [`MessageTransport`] relay that the `forge-shell` IPC layer attaches to
//!   the parent webview.

pub mod bootstrap;
pub mod registry;
pub mod server;

pub use bootstrap::{Bootstrap, BootstrapError, Downloader, HttpDownloader};
pub use registry::{Checksum, Registry, ServerId, ServerSpec};

pub use server::{
    BackoffPolicy, Clock, MessageTransport, Server, ServerError, ServerEvent, StdioTransport,
    SystemClock,
};
