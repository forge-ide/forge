//! F-151 settings IPC tests.
//!
//! Covers `get_settings` + `set_setting` end-to-end through Tauri's
//! `test::get_ipc_response`. Mirrors `tests/approval_commands.rs`
//! (F-036) — same wiring, same test-dir override. The user-scope config dir
//! is redirected via the `webview-test`-gated
//! [`BridgeState::with_test_user_config_dir`] so tests never touch the real
//! platform config dir.
//!
//! F-349: regression tests at the bottom verify that session-* callers have
//! their webview-supplied `workspace_root` replaced by the server-side cache,
//! and that dashboard callers are validated against the workspaces registry.

#![cfg(feature = "webview-test")]

use forge_core::workspaces::{write_workspaces, WorkspaceEntry};
use forge_shell::bridge::SessionConnections;
use forge_shell::ipc::{build_invoke_handler, BridgeState};
use tauri::test::{mock_builder, mock_context, noop_assets, INVOKE_KEY};
use tauri::Manager;
use tempfile::TempDir;

/// Build a mock app with the workspace-root cache primed for `session_id`
/// and the user-config-dir pointed at `user_cfg_dir`.
async fn make_app(
    workspace: &std::path::Path,
    session_id: &str,
    user_cfg_dir: &std::path::Path,
) -> tauri::App<tauri::test::MockRuntime> {
    let connections = SessionConnections::new();
    connections
        .prime_workspace_root_for_test(
            session_id.to_string(),
            std::fs::canonicalize(workspace).expect("canonicalize workspace"),
        )
        .await;
    let app = mock_builder()
        .invoke_handler(build_invoke_handler())
        .build(mock_context(noop_assets()))
        .expect("build mock Tauri app");
    app.manage(BridgeState::with_test_user_config_dir(
        connections,
        user_cfg_dir.to_path_buf(),
    ));
    app
}

/// Build a mock app for dashboard-caller tests: workspaces registry seeded
/// with `workspace_paths`, user-config-dir at `user_cfg_dir`.
async fn make_app_with_registry(
    workspace_paths: &[&std::path::Path],
    user_cfg_dir: &std::path::Path,
) -> (tauri::App<tauri::test::MockRuntime>, TempDir) {
    let registry_dir = TempDir::new().unwrap();
    let toml_path = registry_dir.path().join("workspaces.toml");
    let entries: Vec<WorkspaceEntry> = workspace_paths
        .iter()
        .enumerate()
        .map(|(i, p)| WorkspaceEntry {
            id: forge_core::WorkspaceId::new(),
            path: p.to_path_buf(),
            name: format!("ws-{i}"),
            last_opened: chrono::Utc::now(),
            pinned: false,
        })
        .collect();
    write_workspaces(&toml_path, &entries)
        .await
        .expect("seed workspaces.toml");

    let connections = SessionConnections::new();
    let app = mock_builder()
        .invoke_handler(build_invoke_handler())
        .build(mock_context(noop_assets()))
        .expect("build mock Tauri app");
    app.manage(BridgeState::with_test_user_config_and_workspaces(
        connections,
        user_cfg_dir.to_path_buf(),
        toml_path,
    ));
    (app, registry_dir)
}

fn make_session_window(
    app: &tauri::App<tauri::test::MockRuntime>,
    session_id: &str,
) -> tauri::WebviewWindow<tauri::test::MockRuntime> {
    tauri::WebviewWindowBuilder::new(
        app,
        format!("session-{session_id}"),
        tauri::WebviewUrl::App("index.html".into()),
    )
    .build()
    .expect("mock session window")
}

fn make_dashboard_window(
    app: &tauri::App<tauri::test::MockRuntime>,
) -> tauri::WebviewWindow<tauri::test::MockRuntime> {
    tauri::WebviewWindowBuilder::new(
        app,
        "dashboard",
        tauri::WebviewUrl::App("index.html".into()),
    )
    .build()
    .expect("mock dashboard window")
}

fn invoke_ok(
    window: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    cmd: &str,
    payload: serde_json::Value,
) -> serde_json::Value {
    let res = tauri::test::get_ipc_response(
        window,
        tauri::webview::InvokeRequest {
            cmd: cmd.into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: "http://tauri.localhost".parse().unwrap(),
            body: tauri::ipc::InvokeBody::Json(payload),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        },
    );
    res.expect("invoke returned Ok").deserialize().unwrap()
}

fn invoke_err(
    window: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    cmd: &str,
    payload: serde_json::Value,
) -> String {
    let res = tauri::test::get_ipc_response(
        window,
        tauri::webview::InvokeRequest {
            cmd: cmd.into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: "http://tauri.localhost".parse().unwrap(),
            body: tauri::ipc::InvokeBody::Json(payload),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        },
    );
    match res {
        Ok(ok) => panic!("expected error, got Ok: {ok:?}"),
        Err(serde_json::Value::String(s)) => s,
        Err(other) => other.to_string(),
    }
}

// ---------------------------------------------------------------------------
// get_settings
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn get_settings_returns_defaults_when_no_files() {
    let workspace = TempDir::new().unwrap();
    let user_cfg_dir = TempDir::new().unwrap();

    let sid = "abcdef0123456789";
    let app = make_app(workspace.path(), sid, user_cfg_dir.path()).await;
    let window = make_session_window(&app, sid);

    let result = invoke_ok(
        &window,
        "get_settings",
        serde_json::json!({ "workspaceRoot": workspace.path() }),
    );
    assert_eq!(result["notifications"]["bg_agents"], "toast");
    assert_eq!(result["windows"]["session_mode"], "single");
}

#[tokio::test(flavor = "multi_thread")]
async fn set_setting_workspace_then_get_reflects_value() {
    let workspace = TempDir::new().unwrap();
    let user_cfg_dir = TempDir::new().unwrap();

    let sid = "abcdef0123456789";
    let app = make_app(workspace.path(), sid, user_cfg_dir.path()).await;
    let window = make_session_window(&app, sid);

    let _: serde_json::Value = invoke_ok(
        &window,
        "set_setting",
        serde_json::json!({
            "key": "notifications.bg_agents",
            "value": "os",
            "level": "workspace",
            "workspaceRoot": workspace.path(),
        }),
    );

    let got = invoke_ok(
        &window,
        "get_settings",
        serde_json::json!({ "workspaceRoot": workspace.path() }),
    );
    assert_eq!(got["notifications"]["bg_agents"], "os");
    // Unset fields still default.
    assert_eq!(got["windows"]["session_mode"], "single");

    let on_disk = workspace.path().join(".forge").join("settings.toml");
    assert!(on_disk.exists(), "workspace settings.toml should exist");
}

#[tokio::test(flavor = "multi_thread")]
async fn set_setting_user_writes_under_overridden_dir() {
    let workspace = TempDir::new().unwrap();
    let user_cfg_dir = TempDir::new().unwrap();

    let sid = "abcdef0123456789";
    let app = make_app(workspace.path(), sid, user_cfg_dir.path()).await;
    let window = make_session_window(&app, sid);

    let _: serde_json::Value = invoke_ok(
        &window,
        "set_setting",
        serde_json::json!({
            "key": "windows.session_mode",
            "value": "split",
            "level": "user",
            "workspaceRoot": workspace.path(),
        }),
    );

    let on_disk = user_cfg_dir.path().join("forge").join("settings.toml");
    assert!(on_disk.exists(), "user settings.toml should exist");

    let got = invoke_ok(
        &window,
        "get_settings",
        serde_json::json!({ "workspaceRoot": workspace.path() }),
    );
    assert_eq!(got["windows"]["session_mode"], "split");
}

#[tokio::test(flavor = "multi_thread")]
async fn workspace_overrides_user_on_declared_field_only() {
    let workspace = TempDir::new().unwrap();
    let user_cfg_dir = TempDir::new().unwrap();

    let sid = "abcdef0123456789";
    let app = make_app(workspace.path(), sid, user_cfg_dir.path()).await;
    let window = make_session_window(&app, sid);

    // User sets BOTH fields away from default.
    let _: serde_json::Value = invoke_ok(
        &window,
        "set_setting",
        serde_json::json!({
            "key": "notifications.bg_agents",
            "value": "silent",
            "level": "user",
            "workspaceRoot": workspace.path(),
        }),
    );
    let _: serde_json::Value = invoke_ok(
        &window,
        "set_setting",
        serde_json::json!({
            "key": "windows.session_mode",
            "value": "split",
            "level": "user",
            "workspaceRoot": workspace.path(),
        }),
    );

    // Workspace overrides ONLY notifications.bg_agents.
    let _: serde_json::Value = invoke_ok(
        &window,
        "set_setting",
        serde_json::json!({
            "key": "notifications.bg_agents",
            "value": "os",
            "level": "workspace",
            "workspaceRoot": workspace.path(),
        }),
    );

    let got = invoke_ok(
        &window,
        "get_settings",
        serde_json::json!({ "workspaceRoot": workspace.path() }),
    );
    // Workspace wins on the field it declared.
    assert_eq!(got["notifications"]["bg_agents"], "os");
    // User's other field is preserved through the merge.
    assert_eq!(got["windows"]["session_mode"], "split");
}

#[tokio::test(flavor = "multi_thread")]
async fn set_setting_preserves_sibling_workspace_fields() {
    // The "don't wipe other fields" guarantee from the F-151 design note. Set
    // bg_agents first, then set session_mode, then verify the first write
    // survived.
    let workspace = TempDir::new().unwrap();
    let user_cfg_dir = TempDir::new().unwrap();

    let sid = "abcdef0123456789";
    let app = make_app(workspace.path(), sid, user_cfg_dir.path()).await;
    let window = make_session_window(&app, sid);

    let _: serde_json::Value = invoke_ok(
        &window,
        "set_setting",
        serde_json::json!({
            "key": "notifications.bg_agents",
            "value": "both",
            "level": "workspace",
            "workspaceRoot": workspace.path(),
        }),
    );
    let _: serde_json::Value = invoke_ok(
        &window,
        "set_setting",
        serde_json::json!({
            "key": "windows.session_mode",
            "value": "split",
            "level": "workspace",
            "workspaceRoot": workspace.path(),
        }),
    );

    let got = invoke_ok(
        &window,
        "get_settings",
        serde_json::json!({ "workspaceRoot": workspace.path() }),
    );
    // First write must survive.
    assert_eq!(got["notifications"]["bg_agents"], "both");
    // Second write is visible too.
    assert_eq!(got["windows"]["session_mode"], "split");
}

#[tokio::test(flavor = "multi_thread")]
async fn set_setting_rejects_invalid_value_for_known_key() {
    let workspace = TempDir::new().unwrap();
    let user_cfg_dir = TempDir::new().unwrap();

    let sid = "abcdef0123456789";
    let app = make_app(workspace.path(), sid, user_cfg_dir.path()).await;
    let window = make_session_window(&app, sid);

    // `bg_agents` accepts only the enum strings; an integer must fail.
    let err = invoke_err(
        &window,
        "set_setting",
        serde_json::json!({
            "key": "notifications.bg_agents",
            "value": 42,
            "level": "workspace",
            "workspaceRoot": workspace.path(),
        }),
    );
    assert!(
        err.contains("invalid setting value"),
        "expected validation error, got {err}"
    );
    // On a rejected set, no file should land on disk.
    let on_disk = workspace.path().join(".forge").join("settings.toml");
    assert!(
        !on_disk.exists(),
        "invalid set must not create the settings file"
    );
}

// ---------------------------------------------------------------------------
// Authz: dashboard + session-* allowed, anything else rejected.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn dashboard_window_is_allowed() {
    let workspace = TempDir::new().unwrap();
    let user_cfg_dir = TempDir::new().unwrap();

    let (app, _registry) = make_app_with_registry(&[workspace.path()], user_cfg_dir.path()).await;
    let window = make_dashboard_window(&app);

    let got = invoke_ok(
        &window,
        "get_settings",
        serde_json::json!({ "workspaceRoot": workspace.path() }),
    );
    assert_eq!(got["notifications"]["bg_agents"], "toast");
}

#[tokio::test(flavor = "multi_thread")]
async fn non_session_non_dashboard_window_is_rejected() {
    let workspace = TempDir::new().unwrap();
    let user_cfg_dir = TempDir::new().unwrap();

    let connections = SessionConnections::new();
    let app = mock_builder()
        .invoke_handler(build_invoke_handler())
        .build(mock_context(noop_assets()))
        .expect("build mock Tauri app");
    app.manage(BridgeState::with_test_user_config_dir(
        connections,
        user_cfg_dir.path().to_path_buf(),
    ));

    let window = tauri::WebviewWindowBuilder::new(
        &app,
        "some-other-window",
        tauri::WebviewUrl::App("index.html".into()),
    )
    .build()
    .expect("mock window");

    let err = invoke_err(
        &window,
        "get_settings",
        serde_json::json!({ "workspaceRoot": workspace.path() }),
    );
    assert!(
        err.contains("forbidden"),
        "expected label-mismatch rejection, got {err}"
    );

    let err = invoke_err(
        &window,
        "set_setting",
        serde_json::json!({
            "key": "notifications.bg_agents",
            "value": "os",
            "level": "workspace",
            "workspaceRoot": workspace.path(),
        }),
    );
    assert!(
        err.contains("forbidden"),
        "expected label-mismatch rejection on set_setting, got {err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn oversize_workspace_root_rejected() {
    let user_cfg_dir = TempDir::new().unwrap();
    let connections = SessionConnections::new();
    let app = mock_builder()
        .invoke_handler(build_invoke_handler())
        .build(mock_context(noop_assets()))
        .expect("build mock Tauri app");
    app.manage(BridgeState::with_test_user_config_dir(
        connections,
        user_cfg_dir.path().to_path_buf(),
    ));
    let window = make_session_window(&app, "abcdef0123456789");

    let huge = "a".repeat(4097);
    let err = invoke_err(
        &window,
        "get_settings",
        serde_json::json!({ "workspaceRoot": huge }),
    );
    assert!(
        err.contains("payload too large") && err.contains("workspace_root"),
        "expected workspace_root cap error, got {err}"
    );
}

// ---------------------------------------------------------------------------
// F-349: workspace-root authorization regression tests
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn session_window_forged_workspace_root_is_ignored_for_get() {
    // A session-* webview supplying a forged `workspace_root` must have its
    // value replaced by the server-side cached workspace root.
    let real_ws = TempDir::new().unwrap();
    let forged_ws = TempDir::new().unwrap();
    let user_cfg_dir = TempDir::new().unwrap();

    // Plant a workspace-level settings file in the real workspace only.
    let real_forge_dir = real_ws.path().join(".forge");
    std::fs::create_dir_all(&real_forge_dir).unwrap();
    std::fs::write(
        real_forge_dir.join("settings.toml"),
        "[notifications]\nbg_agents = \"os\"\n",
    )
    .unwrap();

    let sid = "sess-forged-get";
    let app = make_app(real_ws.path(), sid, user_cfg_dir.path()).await;
    let window = make_session_window(&app, sid);

    let forged_root = forged_ws.path().to_string_lossy().to_string();
    let got = invoke_ok(
        &window,
        "get_settings",
        serde_json::json!({ "workspaceRoot": forged_root }),
    );
    // The real workspace's setting is visible — server-side cache won.
    assert_eq!(
        got["notifications"]["bg_agents"], "os",
        "session-* get_settings must read from the cached workspace, not the forged path"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn session_window_forged_workspace_write_lands_in_cached_path() {
    // A session-* webview supplying a forged `workspace_root` to `set_setting`
    // with `level=workspace` must have the write land in the real (cached)
    // workspace, not the forged path.
    let real_ws = TempDir::new().unwrap();
    let forged_ws = TempDir::new().unwrap();
    let user_cfg_dir = TempDir::new().unwrap();

    let sid = "sess-forged-write";
    let app = make_app(real_ws.path(), sid, user_cfg_dir.path()).await;
    let window = make_session_window(&app, sid);

    let forged_root = forged_ws.path().to_string_lossy().to_string();
    let _: serde_json::Value = invoke_ok(
        &window,
        "set_setting",
        serde_json::json!({
            "key": "notifications.bg_agents",
            "value": "os",
            "level": "workspace",
            "workspaceRoot": forged_root,
        }),
    );

    assert!(
        real_ws.path().join(".forge").join("settings.toml").exists(),
        "write must land in the real (cached) workspace"
    );
    assert!(
        !forged_ws.path().join(".forge").exists(),
        "forged workspace path must not have been touched"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dashboard_window_rejects_workspace_root_not_in_registry() {
    let registered_ws = TempDir::new().unwrap();
    let unregistered_ws = TempDir::new().unwrap();
    let user_cfg_dir = TempDir::new().unwrap();

    let (app, _registry) =
        make_app_with_registry(&[registered_ws.path()], user_cfg_dir.path()).await;
    let window = make_dashboard_window(&app);

    let unregistered_root = unregistered_ws.path().to_string_lossy().to_string();
    let err = invoke_err(
        &window,
        "get_settings",
        serde_json::json!({ "workspaceRoot": unregistered_root }),
    );
    assert!(
        err.contains("not in registry"),
        "expected registry-rejection error for dashboard caller, got: {err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dashboard_window_accepts_registered_workspace_root() {
    let workspace = TempDir::new().unwrap();
    let user_cfg_dir = TempDir::new().unwrap();

    let (app, _registry) = make_app_with_registry(&[workspace.path()], user_cfg_dir.path()).await;
    let window = make_dashboard_window(&app);

    let root = workspace.path().to_string_lossy().to_string();
    let got = invoke_ok(
        &window,
        "get_settings",
        serde_json::json!({ "workspaceRoot": root }),
    );
    assert_eq!(got["notifications"]["bg_agents"], "toast");
}
