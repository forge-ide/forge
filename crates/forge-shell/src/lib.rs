#![deny(rustdoc::broken_intra_doc_links, rustdoc::private_intra_doc_links)]
//! forge-shell: Tauri 2 host for the Forge Solid app.
//!
//! Modules:
//! - [`window_spec`]: pure declarative window configuration. Unit-tested.
//! - [`window_manager`]: runtime adapter that applies a `WindowSpec` to a live
//!   `tauri::AppHandle`. Compile-verified; no unit tests (requires a live
//!   webview runtime).
//! - [`dashboard`]: Provider status probe + TTL cache for the Dashboard's
//!   ProviderPanel.
//! - [`dashboard_sessions`]: Dashboard sessions list + open Tauri commands
//!   and their pure helpers. The `collect_sessions` helper and `Pinger` trait
//!   are always compiled so they can be exercised by unit tests under
//!   `--no-default-features`; the `#[tauri::command]` wrappers are gated
//!   behind `webview`.
//!
//! `window_manager` is gated behind the `webview` feature (on by default) so
//! that `window_spec` can be unit-tested on hosts without WebKitGTK via
//! `cargo test -p forge-shell --no-default-features`.
//!
//! # Structured tracing (F-371)
//!
//! All `tracing` emissions from this crate — authz rejections in
//! `ipc::require_window_label` / `ipc::require_window_label_in`, Tauri
//! emit-target failures in [`ipc`], and the terminal / LSP forwarders —
//! use the field-name and target schema pinned in
//! [`forge_session::log_fields`]. That module is the authoritative
//! reference for operators writing log filters; do not introduce new
//! ad-hoc field names at emission sites.

pub mod bridge;
pub mod dashboard;
pub mod dashboard_sessions;
pub mod window_spec;

#[cfg(feature = "webview")]
pub mod ipc;
#[cfg(feature = "webview")]
pub mod window_manager;
