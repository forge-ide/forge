//! F-151 settings IPC tests.
//!
//! Covers `get_settings` + `set_setting` end-to-end through Tauri's
//! `test::get_ipc_response`. Mirrors `tests/approval_commands.rs`
//! (F-036) — same wiring, same test-dir override. The user-scope config dir
//! is redirected via the `webview-test`-gated
//! [`BridgeState::with_test_user_config_dir`] so tests never touch the real
//! platform config dir.

#![cfg(feature = "webview-test")]

use forge_shell::bridge::SessionConnections;
use forge_shell::ipc::{build_invoke_handler, BridgeState};
use tauri::test::{mock_builder, mock_context, noop_assets, INVOKE_KEY};
use tauri::Manager;
use tempfile::TempDir;

fn make_app() -> tauri::App<tauri::test::MockRuntime> {
    mock_builder()
        .invoke_handler(build_invoke_handler())
        .build(mock_context(noop_assets()))
        .expect("build mock Tauri app")
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
    .expect("mock window")
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

    let app = make_app();
    app.manage(BridgeState::with_test_user_config_dir(
        SessionConnections::new(),
        user_cfg_dir.path().to_path_buf(),
    ));
    let window = make_session_window(&app, "abcdef0123456789");

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

    let app = make_app();
    app.manage(BridgeState::with_test_user_config_dir(
        SessionConnections::new(),
        user_cfg_dir.path().to_path_buf(),
    ));
    let window = make_session_window(&app, "abcdef0123456789");

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

    let app = make_app();
    app.manage(BridgeState::with_test_user_config_dir(
        SessionConnections::new(),
        user_cfg_dir.path().to_path_buf(),
    ));
    let window = make_session_window(&app, "abcdef0123456789");

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

    let app = make_app();
    app.manage(BridgeState::with_test_user_config_dir(
        SessionConnections::new(),
        user_cfg_dir.path().to_path_buf(),
    ));
    let window = make_session_window(&app, "abcdef0123456789");

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

    let app = make_app();
    app.manage(BridgeState::with_test_user_config_dir(
        SessionConnections::new(),
        user_cfg_dir.path().to_path_buf(),
    ));
    let window = make_session_window(&app, "abcdef0123456789");

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

    let app = make_app();
    app.manage(BridgeState::with_test_user_config_dir(
        SessionConnections::new(),
        user_cfg_dir.path().to_path_buf(),
    ));
    let window = make_session_window(&app, "abcdef0123456789");

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
// Authz: dashboard + session-* allowed, anything else rejected. Settings are
// user-scoped (not per-session), so there is no session-to-session label
// check — F-036 enforces the same invariant via `require_window_label_in`.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn dashboard_window_is_allowed() {
    let workspace = TempDir::new().unwrap();
    let user_cfg_dir = TempDir::new().unwrap();

    let app = make_app();
    app.manage(BridgeState::with_test_user_config_dir(
        SessionConnections::new(),
        user_cfg_dir.path().to_path_buf(),
    ));
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

    let app = make_app();
    app.manage(BridgeState::with_test_user_config_dir(
        SessionConnections::new(),
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
    let app = make_app();
    app.manage(BridgeState::with_test_user_config_dir(
        SessionConnections::new(),
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
