//! Runtime adapter that materializes a [`WindowSpec`] on a live Tauri app.
//!
//! Not unit-tested — exercising this path requires a live webview runtime
//! (`WebviewWindowBuilder::build()` talks to wry/WebKitGTK). CI
//! compile-verifies via `cargo check --features webview`; the manual smoke
//! test is `cargo run -p forge-shell`.

use anyhow::{Context, Result};
use tauri::{AppHandle, Manager, Runtime, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

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

/// Entry point invoked from `main`. Builds the Tauri app and opens the
/// Dashboard on setup.
pub fn run() -> Result<()> {
    tauri::Builder::default()
        .setup(|app| {
            let manager = WindowManager::new(app.handle().clone());
            manager.open_dashboard()?;
            Ok(())
        })
        .run(tauri::generate_context!("src-tauri/tauri.conf.json"))
        .context("tauri runtime exited with an error")
}
