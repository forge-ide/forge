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
        // F-138: OS-notification branch of `notifications.bg_agents`.
        // `.plugin(init())` registers `plugin:notification|*` commands that
        // the webview calls via `@tauri-apps/plugin-notification`. The
        // capability file grants the browser-facing permissions; see
        // `crates/forge-shell/capabilities/default.json`.
        .plugin(tauri_plugin_notification::init())
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
            crate::ipc::get_persistent_approvals,
            crate::ipc::save_approval,
            crate::ipc::remove_approval,
            crate::ipc::terminal_spawn,
            crate::ipc::terminal_write,
            crate::ipc::terminal_resize,
            crate::ipc::terminal_kill,
            crate::ipc::read_file,
            crate::ipc::write_file,
            crate::ipc::tree,
            crate::ipc::lsp_start,
            crate::ipc::lsp_stop,
            crate::ipc::lsp_send,
            // F-137: background-agent lifecycle commands. Registered here
            // alongside F-138's `stop_background_agent` so the shell binary
            // exposes the full quartet in production. Tests reach the same
            // commands through `build_invoke_handler` in `ipc.rs`; until this
            // list is deduplicated against that helper, both call sites must
            // be updated in lockstep.
            crate::ipc::start_background_agent,
            crate::ipc::promote_background_agent,
            crate::ipc::list_background_agents,
            // F-138: stop completes the quartet.
            crate::ipc::stop_background_agent,
            // F-151: persistent settings store. Required by F-138's
            // `notifications.bg_agents` read; without it, `getSettings`
            // fails and the status-bar notification handler falls back to
            // the default `toast` mode.
            crate::ipc::get_settings,
            crate::ipc::set_setting,
            // F-132: MCP commands — list / toggle / import.
            crate::ipc::list_mcp_servers,
            crate::ipc::toggle_mcp_server,
            crate::ipc::import_mcp_config,
        ])
        .setup(|app| {
            crate::ipc::manage_bridge(&app.handle().clone());
            crate::ipc::manage_terminals(&app.handle().clone());
            crate::ipc::manage_lsp(&app.handle().clone());
            // F-137 / F-138: background-agent lifecycle state. Each session's
            // `BackgroundAgentRegistry` is lazily populated by
            // `resolve_bg_session` on first invoke; the state container has
            // to be managed up-front so the `State<'_, BgAgentState>` extractor
            // on each command doesn't panic.
            crate::ipc::manage_bg_agents(&app.handle().clone());
            // F-155: MCP commands now dispatch over the session UDS
            // bridge; no shell-side manager state to initialise.
            let manager = WindowManager::new(app.handle().clone());
            manager.open_dashboard()?;
            Ok(())
        })
        .run(tauri::generate_context!("tauri.conf.json"))
        .context("tauri runtime exited with an error")
}
