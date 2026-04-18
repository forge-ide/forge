//! forge-shell: Tauri 2 host for the Forge Solid app.
//!
//! Split into two modules:
//! - [`window_spec`]: pure declarative window configuration. Unit-tested.
//! - [`window_manager`]: runtime adapter that applies a `WindowSpec` to a live
//!   `tauri::AppHandle`. Compile-verified; no unit tests (requires a live
//!   webview runtime).
//!
//! `window_manager` is gated behind the `webview` feature (on by default) so
//! that `window_spec` can be unit-tested on hosts without WebKitGTK via
//! `cargo test -p forge-shell --no-default-features`.

pub mod window_spec;

#[cfg(feature = "webview")]
pub mod window_manager;
