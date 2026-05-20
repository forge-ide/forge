//! Tests for the `register_workspace` IPC command.
//!
//! Backs the dashboard "Open workspace" picker on `EnabledAssetsCard`. Without
//! this seed step, the picker's downstream `list_skills` / `list_mcp_servers`
//! / `list_agents` calls trip `resolve_workspace_root_for_command`'s
//! defense-in-depth gate ("workspace_root not in registry").
//!
//! Coverage:
//!  * Dashboard label accepted; non-dashboard labels rejected.
//!  * Missing on-disk path is rejected with a `register_workspace:` prefix.
//!  * A valid path is canonicalized and appended to `workspaces.toml`, then
//!    immediately accepted by `list_skills` (proves the gate sees the entry).
//!  * Idempotent — registering the same path twice keeps a single entry.

#![cfg(feature = "webview-test")]

use forge_core::workspaces::read_workspaces;
use forge_shell::bridge::SessionConnections;
use forge_shell::ipc::{build_invoke_handler, BridgeState};
use serde_json::{json, Value};
use tauri::test::{mock_builder, mock_context, noop_assets, INVOKE_KEY};
use tauri::Manager;
use tempfile::TempDir;

async fn make_app() -> (
    tauri::App<tauri::test::MockRuntime>,
    TempDir,
    TempDir,
    std::path::PathBuf,
) {
    let registry_dir = TempDir::new().unwrap();
    let user_cfg_dir = TempDir::new().unwrap();
    let toml_path = registry_dir.path().join("workspaces.toml");

    let connections = SessionConnections::new();
    let app = mock_builder()
        .invoke_handler(build_invoke_handler())
        .build(mock_context(noop_assets()))
        .expect("build mock Tauri app");
    app.manage(BridgeState::with_test_user_config_and_workspaces(
        connections,
        user_cfg_dir.path().to_path_buf(),
        toml_path.clone(),
    ));
    (app, registry_dir, user_cfg_dir, toml_path)
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

fn make_session_window(
    app: &tauri::App<tauri::test::MockRuntime>,
    label: &str,
) -> tauri::WebviewWindow<tauri::test::MockRuntime> {
    tauri::WebviewWindowBuilder::new(app, label, tauri::WebviewUrl::App("index.html".into()))
        .build()
        .expect("mock session window")
}

fn invoke(
    window: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    cmd: &str,
    payload: Value,
) -> Result<Value, Value> {
    tauri::test::get_ipc_response(
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
    )
    .map(|ok| ok.deserialize().unwrap())
}

#[tokio::test(flavor = "multi_thread")]
async fn register_workspace_accepts_dashboard_label() {
    let workspace = TempDir::new().unwrap();
    let canonical = std::fs::canonicalize(workspace.path()).expect("canonicalize");

    let (app, _reg, _cfg, toml_path) = make_app().await;
    let window = make_dashboard_window(&app);

    let returned: Value = invoke(
        &window,
        "register_workspace",
        json!({ "workspaceRoot": canonical }),
    )
    .expect("dashboard call should succeed");

    assert_eq!(
        returned.as_str().unwrap(),
        canonical.to_string_lossy(),
        "command should echo the canonical path"
    );

    let entries = read_workspaces(&toml_path).await.unwrap();
    assert_eq!(entries.len(), 1, "one entry written");
    assert_eq!(entries[0].path, canonical);
}

#[tokio::test(flavor = "multi_thread")]
async fn register_workspace_rejects_session_label() {
    let workspace = TempDir::new().unwrap();
    let canonical = std::fs::canonicalize(workspace.path()).expect("canonicalize");

    let (app, _reg, _cfg, toml_path) = make_app().await;
    let window = make_session_window(&app, "session-deadbeefdeadbeef");

    let err = invoke(
        &window,
        "register_workspace",
        json!({ "workspaceRoot": canonical }),
    )
    .expect_err("session-* window must be rejected");

    let msg = err.as_str().unwrap_or("");
    assert!(
        msg.contains("forbidden: window label mismatch"),
        "session-* label should fail the authz gate: {msg}"
    );

    let entries = read_workspaces(&toml_path).await.unwrap();
    assert!(entries.is_empty(), "rejected call must not seed registry");
}

#[tokio::test(flavor = "multi_thread")]
async fn register_workspace_rejects_missing_path() {
    let (app, _reg, _cfg, toml_path) = make_app().await;
    let window = make_dashboard_window(&app);

    let err = invoke(
        &window,
        "register_workspace",
        json!({ "workspaceRoot": "/this/path/does/not/exist/forge-test" }),
    )
    .expect_err("missing path must be rejected");

    let msg = err.as_str().unwrap_or("");
    assert!(
        msg.contains("register_workspace:") && msg.contains("not found on disk"),
        "error should mention not-found: {msg}"
    );

    let entries = read_workspaces(&toml_path).await.unwrap();
    assert!(entries.is_empty(), "rejected call must not seed registry");
}

#[tokio::test(flavor = "multi_thread")]
async fn register_workspace_is_idempotent() {
    let workspace = TempDir::new().unwrap();
    let canonical = std::fs::canonicalize(workspace.path()).expect("canonicalize");

    let (app, _reg, _cfg, toml_path) = make_app().await;
    let window = make_dashboard_window(&app);

    let _ = invoke(
        &window,
        "register_workspace",
        json!({ "workspaceRoot": canonical }),
    )
    .expect("first call ok");
    let _ = invoke(
        &window,
        "register_workspace",
        json!({ "workspaceRoot": canonical }),
    )
    .expect("second call ok");

    let entries = read_workspaces(&toml_path).await.unwrap();
    assert_eq!(entries.len(), 1, "duplicate entry must not be appended");
}

#[tokio::test(flavor = "multi_thread")]
async fn register_workspace_unblocks_list_skills_gate() {
    // Demonstrates the bug fix end-to-end: a freshly picked workspace
    // resolves successfully through `list_skills` after `register_workspace`,
    // whereas before this command existed the dashboard caller's gate would
    // reject with "workspace_root not in registry".
    let workspace = TempDir::new().unwrap();
    let canonical = std::fs::canonicalize(workspace.path()).expect("canonicalize");

    let (app, _reg, _cfg, _toml) = make_app().await;
    let window = make_dashboard_window(&app);

    // Sanity: without registration, list_skills rejects.
    let pre_err = invoke(
        &window,
        "list_skills",
        json!({
            "workspaceRoot": canonical,
            "scope": { "type": "SessionWide" },
        }),
    )
    .expect_err("pre-registration list_skills must fail");
    assert!(
        pre_err
            .as_str()
            .unwrap_or("")
            .contains("workspace_root not in registry"),
        "pre-registration error must be the registry-gate message: {pre_err:?}"
    );

    // Register the workspace.
    let _ = invoke(
        &window,
        "register_workspace",
        json!({ "workspaceRoot": canonical }),
    )
    .expect("register ok");

    // Now list_skills accepts the same workspace_root.
    let post = invoke(
        &window,
        "list_skills",
        json!({
            "workspaceRoot": canonical,
            "scope": { "type": "SessionWide" },
        }),
    )
    .expect("post-registration list_skills must succeed");
    assert!(post.is_array(), "list_skills returns an array");
}
