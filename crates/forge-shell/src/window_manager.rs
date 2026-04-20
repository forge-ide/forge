//! Runtime adapter that materializes a [`WindowSpec`] on a live Tauri app.
//!
//! Not unit-tested — exercising this path requires a live webview runtime
//! (`WebviewWindowBuilder::build()` talks to wry/WebKitGTK). CI
//! compile-verifies via `cargo check --features webview`; the manual smoke
//! test is `cargo run -p forge-shell`.

use anyhow::{Context, Result};
use tauri::{AppHandle, Manager, Runtime, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::dashboard::{self, ProviderStatusCache, CACHE_TTL};
use crate::window_spec::WindowSpec;

/// Opens and manages Forge windows on a live `AppHandle`.
pub struct WindowManager<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> WindowManager<R> {
    pub fn new(app: AppHandle<R>) -> Self {
        Self { app }
    }

    /// Opens the Dashboard window, or focuses it if already open.
    pub fn open_dashboard(&self) -> Result<WebviewWindow<R>> {
        self.open(WindowSpec::dashboard())
    }

    /// Opens a blank Session window for `id`.
    ///
    /// Scaffold only — F-024 wires the session route and content. No IPC to
    /// `forge-session` yet (F-020).
    pub fn open_session(&self, id: &str) -> Result<WebviewWindow<R>> {
        self.open(WindowSpec::session(id))
    }

    fn open(&self, spec: WindowSpec) -> Result<WebviewWindow<R>> {
        if let Some(existing) = self.app.get_webview_window(&spec.label) {
            existing
                .set_focus()
                .with_context(|| format!("focus existing window `{}`", spec.label))?;
            return Ok(existing);
        }

        let mut builder =
            WebviewWindowBuilder::new(&self.app, &spec.label, WebviewUrl::App(spec.url.into()))
                .title(&spec.title)
                .inner_size(spec.width, spec.height)
                .min_inner_size(spec.min_width, spec.min_height)
                .resizable(spec.resizable)
                .decorations(spec.decorations);
        if spec.center {
            builder = builder.center();
        }
        builder
            .build()
            .with_context(|| format!("build window `{}`", spec.label))
    }
}

/// Entry point invoked from `main`. Builds the Tauri app, registers the
/// session IPC bridge + dashboard commands, and opens the Dashboard on setup.
pub fn run() -> Result<()> {
    tauri::Builder::default()
        .manage(ProviderStatusCache::new(CACHE_TTL))
        .invoke_handler(tauri::generate_handler![
            dashboard::provider_status,
            crate::dashboard_sessions::session_list,
            crate::dashboard_sessions::open_session,
            crate::ipc::session_hello,
            crate::ipc::session_subscribe,
            crate::ipc::session_send_message,
            crate::ipc::session_approve_tool,
            crate::ipc::session_reject_tool,
        ])
        .setup(|app| {
            crate::ipc::manage_bridge(&app.handle().clone());
            let manager = WindowManager::new(app.handle().clone());
            manager.open_dashboard()?;
            Ok(())
        })
        .run(tauri::generate_context!("tauri.conf.json"))
        .context("tauri runtime exited with an error")
}
