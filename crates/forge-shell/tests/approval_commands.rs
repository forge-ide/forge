//! F-036 persistent-approval Tauri command tests.
//!
//! Covers the three new commands — `get_persistent_approvals`, `save_approval`,
//! `remove_approval` — end-to-end through Tauri's `test::get_ipc_response`
//! machinery. The user-scope config dir is overridden via the
//! `webview-test`-gated [`BridgeState::with_test_user_config_dir`] so tests
//! never touch the real `{config_dir}/forge/approvals.toml`.
//!
//! F-349: regression tests at the bottom verify that session-* callers have
//! their webview-supplied `workspace_root` silently replaced by the
//! server-side cached value, and that dashboard callers are validated against
//! the workspaces registry.
//!
//! Wiring mirrors `tests/ipc_commands.rs` (F-020 / F-052): `mock_builder()` +
//! `WebviewWindowBuilder` with explicit labels and `INVOKE_KEY` payload.

#![cfg(feature = "webview-test")]

use forge_core::workspaces::{write_workspaces, WorkspaceEntry};
use forge_shell::bridge::SessionConnections;
use forge_shell::ipc::{build_invoke_handler, BridgeState};
use tauri::test::{mock_builder, mock_context, noop_assets, INVOKE_KEY};
use tauri::Manager;
use tempfile::TempDir;

/// Build a Tauri mock app with the workspace-root cache primed for `session_id`
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

    // BridgeState that sets BOTH test overrides. We build it manually.
    let connections = SessionConnections::new();
    let app = mock_builder()
        .invoke_handler(build_invoke_handler())
        .build(mock_context(noop_assets()))
        .expect("build mock Tauri app");
    // We need both user-config-dir and workspaces-toml overrides.
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
// Functional correctness (session-* callers, primed cache)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn get_persistent_approvals_returns_empty_when_no_files() {
    let workspace = TempDir::new().unwrap();
    let user_cfg_dir = TempDir::new().unwrap();

    let sid = "abcdef0123456789";
    let app = make_app(workspace.path(), sid, user_cfg_dir.path()).await;
    let window = make_session_window(&app, sid);

    let result = invoke_ok(
        &window,
        "get_persistent_approvals",
        serde_json::json!({ "workspaceRoot": workspace.path() }),
    );
    let arr = result.as_array().unwrap();
    assert!(arr.is_empty(), "expected empty list, got {arr:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn save_approval_then_get_returns_workspace_entry() {
    let workspace = TempDir::new().unwrap();
    let user_cfg_dir = TempDir::new().unwrap();

    let sid = "abcdef0123456789";
    let app = make_app(workspace.path(), sid, user_cfg_dir.path()).await;
    let window = make_session_window(&app, sid);

    let entry = serde_json::json!({
        "scope_key": "tool:fs.write",
        "tool_name": "fs.write",
        "label": "this tool",
    });

    let _: serde_json::Value = invoke_ok(
        &window,
        "save_approval",
        serde_json::json!({
            "entry": entry,
            "level": "workspace",
            "workspaceRoot": workspace.path(),
        }),
    );

    let result = invoke_ok(
        &window,
        "get_persistent_approvals",
        serde_json::json!({ "workspaceRoot": workspace.path() }),
    );
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["scope_key"], "tool:fs.write");
    assert_eq!(arr[0]["tool_name"], "fs.write");
    assert_eq!(arr[0]["label"], "this tool");
    assert_eq!(arr[0]["level"], "workspace");

    // File exists on disk at the expected path.
    let on_disk = workspace.path().join(".forge").join("approvals.toml");
    assert!(on_disk.exists(), "workspace approvals.toml should exist");
}

#[tokio::test(flavor = "multi_thread")]
async fn save_approval_writes_user_config_to_overridden_dir() {
    let workspace = TempDir::new().unwrap();
    let user_cfg_dir = TempDir::new().unwrap();

    let sid = "abcdef0123456789";
    let app = make_app(workspace.path(), sid, user_cfg_dir.path()).await;
    let window = make_session_window(&app, sid);

    let entry = serde_json::json!({
        "scope_key": "tool:shell.exec",
        "tool_name": "shell.exec",
        "label": "this tool",
    });

    let _: serde_json::Value = invoke_ok(
        &window,
        "save_approval",
        serde_json::json!({
            "entry": entry,
            "level": "user",
            "workspaceRoot": workspace.path(),
        }),
    );

    let on_disk = user_cfg_dir.path().join("forge").join("approvals.toml");
    assert!(on_disk.exists(), "user approvals.toml should exist");

    let result = invoke_ok(
        &window,
        "get_persistent_approvals",
        serde_json::json!({ "workspaceRoot": workspace.path() }),
    );
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["level"], "user");
    assert_eq!(arr[0]["scope_key"], "tool:shell.exec");
}

#[tokio::test(flavor = "multi_thread")]
async fn workspace_wins_over_user_on_collision() {
    let workspace = TempDir::new().unwrap();
    let user_cfg_dir = TempDir::new().unwrap();

    let sid = "abcdef0123456789";
    let app = make_app(workspace.path(), sid, user_cfg_dir.path()).await;
    let window = make_session_window(&app, sid);

    // Save the same key under both tiers with distinct labels.
    let user_entry = serde_json::json!({
        "scope_key": "tool:fs.write",
        "tool_name": "fs.write",
        "label": "from user",
    });
    let workspace_entry = serde_json::json!({
        "scope_key": "tool:fs.write",
        "tool_name": "fs.write",
        "label": "from workspace",
    });

    let _: serde_json::Value = invoke_ok(
        &window,
        "save_approval",
        serde_json::json!({
            "entry": user_entry,
            "level": "user",
            "workspaceRoot": workspace.path(),
        }),
    );
    let _: serde_json::Value = invoke_ok(
        &window,
        "save_approval",
        serde_json::json!({
            "entry": workspace_entry,
            "level": "workspace",
            "workspaceRoot": workspace.path(),
        }),
    );

    let result = invoke_ok(
        &window,
        "get_persistent_approvals",
        serde_json::json!({ "workspaceRoot": workspace.path() }),
    );
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1, "workspace should suppress the user entry");
    assert_eq!(arr[0]["level"], "workspace");
    assert_eq!(arr[0]["label"], "from workspace");
}

#[tokio::test(flavor = "multi_thread")]
async fn remove_approval_workspace_drops_entry() {
    let workspace = TempDir::new().unwrap();
    let user_cfg_dir = TempDir::new().unwrap();

    let sid = "abcdef0123456789";
    let app = make_app(workspace.path(), sid, user_cfg_dir.path()).await;
    let window = make_session_window(&app, sid);

    let entry = serde_json::json!({
        "scope_key": "file:fs.edit:/src/foo.ts",
        "tool_name": "fs.edit",
        "label": "this file",
    });
    let _: serde_json::Value = invoke_ok(
        &window,
        "save_approval",
        serde_json::json!({
            "entry": entry,
            "level": "workspace",
            "workspaceRoot": workspace.path(),
        }),
    );

    let _: serde_json::Value = invoke_ok(
        &window,
        "remove_approval",
        serde_json::json!({
            "scopeKey": "file:fs.edit:/src/foo.ts",
            "level": "workspace",
            "workspaceRoot": workspace.path(),
        }),
    );

    let result = invoke_ok(
        &window,
        "get_persistent_approvals",
        serde_json::json!({ "workspaceRoot": workspace.path() }),
    );
    let arr = result.as_array().unwrap();
    assert!(arr.is_empty(), "removed entry must not reappear");
}

#[tokio::test(flavor = "multi_thread")]
async fn remove_approval_user_drops_only_user_tier() {
    let workspace = TempDir::new().unwrap();
    let user_cfg_dir = TempDir::new().unwrap();

    let sid = "abcdef0123456789";
    let app = make_app(workspace.path(), sid, user_cfg_dir.path()).await;
    let window = make_session_window(&app, sid);

    // Two different keys, one per tier.
    let user_entry = serde_json::json!({
        "scope_key": "tool:shell.exec",
        "tool_name": "shell.exec",
        "label": "this tool",
    });
    let workspace_entry = serde_json::json!({
        "scope_key": "tool:fs.write",
        "tool_name": "fs.write",
        "label": "this tool",
    });
    let _: serde_json::Value = invoke_ok(
        &window,
        "save_approval",
        serde_json::json!({
            "entry": user_entry,
            "level": "user",
            "workspaceRoot": workspace.path(),
        }),
    );
    let _: serde_json::Value = invoke_ok(
        &window,
        "save_approval",
        serde_json::json!({
            "entry": workspace_entry,
            "level": "workspace",
            "workspaceRoot": workspace.path(),
        }),
    );

    // Remove only the user-tier entry.
    let _: serde_json::Value = invoke_ok(
        &window,
        "remove_approval",
        serde_json::json!({
            "scopeKey": "tool:shell.exec",
            "level": "user",
            "workspaceRoot": workspace.path(),
        }),
    );

    let result = invoke_ok(
        &window,
        "get_persistent_approvals",
        serde_json::json!({ "workspaceRoot": workspace.path() }),
    );
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["scope_key"], "tool:fs.write");
    assert_eq!(arr[0]["level"], "workspace");
}

#[tokio::test(flavor = "multi_thread")]
async fn session_level_save_is_noop() {
    let workspace = TempDir::new().unwrap();
    let user_cfg_dir = TempDir::new().unwrap();

    let sid = "abcdef0123456789";
    let app = make_app(workspace.path(), sid, user_cfg_dir.path()).await;
    let window = make_session_window(&app, sid);

    let entry = serde_json::json!({
        "scope_key": "tool:fs.write",
        "tool_name": "fs.write",
        "label": "this tool",
    });
    // session-level save should succeed but never write a file.
    let _: serde_json::Value = invoke_ok(
        &window,
        "save_approval",
        serde_json::json!({
            "entry": entry,
            "level": "session",
            "workspaceRoot": workspace.path(),
        }),
    );

    let workspace_file = workspace.path().join(".forge").join("approvals.toml");
    let user_file = user_cfg_dir.path().join("forge").join("approvals.toml");
    assert!(
        !workspace_file.exists(),
        "session-level save must not write workspace file"
    );
    assert!(
        !user_file.exists(),
        "session-level save must not write user file"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dashboard_window_is_allowed() {
    let workspace = TempDir::new().unwrap();
    let user_cfg_dir = TempDir::new().unwrap();

    // Dashboard caller needs the registry.
    let (app, _registry) =
        make_app_with_registry(&[workspace.path()], user_cfg_dir.path()).await;
    let window = make_dashboard_window(&app);
    let result = invoke_ok(
        &window,
        "get_persistent_approvals",
        serde_json::json!({ "workspaceRoot": workspace.path() }),
    );
    assert!(result.as_array().unwrap().is_empty());
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
        "get_persistent_approvals",
        serde_json::json!({ "workspaceRoot": workspace.path() }),
    );
    assert!(
        err.contains("forbidden"),
        "expected label-mismatch rejection, got {err}"
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
        "get_persistent_approvals",
        serde_json::json!({ "workspaceRoot": huge }),
    );
    assert!(
        err.contains("payload too large") && err.contains("workspace_root"),
        "expected workspace_root cap error, got {err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn upsert_replaces_existing_entry_in_place() {
    let workspace = TempDir::new().unwrap();
    let user_cfg_dir = TempDir::new().unwrap();

    let sid = "abcdef0123456789";
    let app = make_app(workspace.path(), sid, user_cfg_dir.path()).await;
    let window = make_session_window(&app, sid);

    let first = serde_json::json!({
        "scope_key": "tool:fs.write",
        "tool_name": "fs.write",
        "label": "first",
    });
    let second = serde_json::json!({
        "scope_key": "tool:fs.write",
        "tool_name": "fs.write",
        "label": "second",
    });
    for entry in [first, second] {
        let _: serde_json::Value = invoke_ok(
            &window,
            "save_approval",
            serde_json::json!({
                "entry": entry,
                "level": "workspace",
                "workspaceRoot": workspace.path(),
            }),
        );
    }
    let result = invoke_ok(
        &window,
        "get_persistent_approvals",
        serde_json::json!({ "workspaceRoot": workspace.path() }),
    );
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1, "upsert must not duplicate the same key");
    assert_eq!(arr[0]["label"], "second");
}

// ---------------------------------------------------------------------------
// F-349: workspace-root authorization regression tests
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn session_window_forged_workspace_root_is_ignored() {
    // A session-* webview supplying a forged `workspace_root` must have its
    // supplied value ignored in favour of the server-side cached value.
    let real_ws = TempDir::new().unwrap();
    let forged_ws = TempDir::new().unwrap();
    let user_cfg_dir = TempDir::new().unwrap();

    let sid = "sess-forged";
    let app = make_app(real_ws.path(), sid, user_cfg_dir.path()).await;
    let window = make_session_window(&app, sid);

    // Plant an approval entry directly in the real workspace file so we can
    // distinguish which path was actually read.
    let real_forge_dir = real_ws.path().join(".forge");
    std::fs::create_dir_all(&real_forge_dir).unwrap();
    std::fs::write(
        real_forge_dir.join("approvals.toml"),
        r#"[[entries]]
scope_key = "tool:fs.write"
tool_name = "fs.write"
label = "from-real-workspace"
"#,
    )
    .unwrap();

    // Invoke with the forged workspace root.
    let forged_root = forged_ws.path().to_string_lossy().to_string();
    let result = invoke_ok(
        &window,
        "get_persistent_approvals",
        serde_json::json!({ "workspaceRoot": forged_root }),
    );
    let arr = result.as_array().unwrap();
    // The real workspace entry is visible — proving the server-side cache won.
    assert_eq!(arr.len(), 1, "must read from the cached (real) workspace");
    assert_eq!(arr[0]["scope_key"], "tool:fs.write");

    // The forged workspace must have no side-effects on disk.
    assert!(
        !forged_ws.path().join(".forge").exists(),
        "forged workspace path must not have been touched"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn session_window_forged_workspace_write_lands_in_cached_path() {
    // A session-* webview supplying a forged `workspace_root` to `save_approval`
    // with `level=workspace` must have the write land in the real (cached)
    // workspace, not in the forged path.
    let real_ws = TempDir::new().unwrap();
    let forged_ws = TempDir::new().unwrap();
    let user_cfg_dir = TempDir::new().unwrap();

    let sid = "sess-write-forged";
    let app = make_app(real_ws.path(), sid, user_cfg_dir.path()).await;
    let window = make_session_window(&app, sid);

    let forged_root = forged_ws.path().to_string_lossy().to_string();
    let _: serde_json::Value = invoke_ok(
        &window,
        "save_approval",
        serde_json::json!({
            "entry": {
                "scope_key": "tool:fs.write",
                "tool_name": "fs.write",
                "label": "test",
            },
            "level": "workspace",
            "workspaceRoot": forged_root,
        }),
    );

    // File must appear in the real workspace, not the forged one.
    assert!(
        real_ws.path().join(".forge").join("approvals.toml").exists(),
        "write must land in the real (cached) workspace"
    );
    assert!(
        !forged_ws.path().join(".forge").exists(),
        "forged workspace path must not have been touched"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dashboard_window_rejects_workspace_root_not_in_registry() {
    // A dashboard caller must be rejected when the supplied path is not in the
    // workspaces registry.
    let registered_ws = TempDir::new().unwrap();
    let unregistered_ws = TempDir::new().unwrap();
    let user_cfg_dir = TempDir::new().unwrap();

    let (app, _registry) =
        make_app_with_registry(&[registered_ws.path()], user_cfg_dir.path()).await;
    let window = make_dashboard_window(&app);

    let unregistered_root = unregistered_ws.path().to_string_lossy().to_string();
    let err = invoke_err(
        &window,
        "get_persistent_approvals",
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

    let (app, _registry) =
        make_app_with_registry(&[workspace.path()], user_cfg_dir.path()).await;
    let window = make_dashboard_window(&app);

    let root = workspace.path().to_string_lossy().to_string();
    let result = invoke_ok(
        &window,
        "get_persistent_approvals",
        serde_json::json!({ "workspaceRoot": root }),
    );
    // Empty — confirms the command executed without error.
    assert!(result.as_array().unwrap().is_empty());
}
