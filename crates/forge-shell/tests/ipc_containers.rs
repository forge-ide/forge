//! F-597 container-lifecycle IPC tests.
//!
//! Covers the four registry/dashboard-scoped commands that don't shell
//! out to a real `podman` binary: `list_active_containers`,
//! `detect_container_runtime` is exercised at unit-level (the live probe
//! depends on a host-installed podman), and `stop_container` /
//! `remove_container` / `container_logs` are exercised via their
//! authz-rejection paths plus the registry-side helper APIs.
//!
//! End-to-end coverage of `stop`/`remove`/`logs` against a real
//! `PodmanRuntime` is deferred to the existing `forge-oci`
//! `podman_full_lifecycle_against_alpine` ignored test (the same path
//! the dashboard ultimately invokes).

#![cfg(feature = "webview-test")]

use forge_core::workspaces::{write_workspaces, WorkspaceEntry};
use forge_shell::bridge::SessionConnections;
use forge_shell::containers_ipc::{
    make_container_info, ContainerRegistryState, MAX_CONTAINER_ID_BYTES,
};
use forge_shell::ipc::{build_invoke_handler, BridgeState};
use tauri::test::{mock_builder, mock_context, noop_assets, INVOKE_KEY};
use tauri::Manager;
use tempfile::TempDir;

async fn make_app(
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
    // Container registry must be managed up-front so the State<'_,
    // ContainerRegistryState> extractor on each command doesn't panic.
    app.manage(ContainerRegistryState::new());
    (app, registry_dir)
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

#[tokio::test(flavor = "multi_thread")]
async fn list_active_containers_returns_empty_when_registry_empty() {
    let workspace = TempDir::new().unwrap();
    let user_cfg_dir = TempDir::new().unwrap();
    let (app, _registry) = make_app(&[workspace.path()], user_cfg_dir.path()).await;
    let window = make_dashboard_window(&app);

    let result = invoke_ok(&window, "list_active_containers", serde_json::json!({}));
    let entries = result.as_array().expect("array");
    assert!(entries.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn list_active_containers_emits_registered_entries() {
    let workspace = TempDir::new().unwrap();
    let user_cfg_dir = TempDir::new().unwrap();
    let (app, _registry) = make_app(&[workspace.path()], user_cfg_dir.path()).await;
    let registry = app.state::<ContainerRegistryState>();
    registry
        .register(make_container_info(
            "sess-1",
            "cid-aaaa",
            "alpine:3.19",
            chrono::Utc::now(),
        ))
        .await;
    let window = make_dashboard_window(&app);

    let result = invoke_ok(&window, "list_active_containers", serde_json::json!({}));
    let entries = result.as_array().expect("array");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["container_id"], "cid-aaaa");
    assert_eq!(entries[0]["session_id"], "sess-1");
    assert_eq!(entries[0]["image"], "alpine:3.19");
    assert_eq!(entries[0]["stopped"], false);
}

#[tokio::test(flavor = "multi_thread")]
async fn list_active_containers_rejects_session_callers() {
    let workspace = TempDir::new().unwrap();
    let user_cfg_dir = TempDir::new().unwrap();
    let (app, _registry) = make_app(&[workspace.path()], user_cfg_dir.path()).await;
    let window = make_session_window(&app, "abcdef0123456789");

    let err = invoke_err(&window, "list_active_containers", serde_json::json!({}));
    assert!(err.contains("forbidden"), "got: {err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_container_rejects_session_callers() {
    let workspace = TempDir::new().unwrap();
    let user_cfg_dir = TempDir::new().unwrap();
    let (app, _registry) = make_app(&[workspace.path()], user_cfg_dir.path()).await;
    let window = make_session_window(&app, "abcdef0123456789");

    let err = invoke_err(
        &window,
        "stop_container",
        serde_json::json!({ "containerId": "cid" }),
    );
    assert!(err.contains("forbidden"), "got: {err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn remove_container_rejects_session_callers() {
    let workspace = TempDir::new().unwrap();
    let user_cfg_dir = TempDir::new().unwrap();
    let (app, _registry) = make_app(&[workspace.path()], user_cfg_dir.path()).await;
    let window = make_session_window(&app, "abcdef0123456789");

    let err = invoke_err(
        &window,
        "remove_container",
        serde_json::json!({ "containerId": "cid" }),
    );
    assert!(err.contains("forbidden"), "got: {err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn container_logs_rejects_session_callers() {
    let workspace = TempDir::new().unwrap();
    let user_cfg_dir = TempDir::new().unwrap();
    let (app, _registry) = make_app(&[workspace.path()], user_cfg_dir.path()).await;
    let window = make_session_window(&app, "abcdef0123456789");

    let err = invoke_err(
        &window,
        "container_logs",
        serde_json::json!({ "containerId": "cid" }),
    );
    assert!(err.contains("forbidden"), "got: {err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn detect_container_runtime_rejects_session_callers() {
    let workspace = TempDir::new().unwrap();
    let user_cfg_dir = TempDir::new().unwrap();
    let (app, _registry) = make_app(&[workspace.path()], user_cfg_dir.path()).await;
    let window = make_session_window(&app, "abcdef0123456789");

    let err = invoke_err(&window, "detect_container_runtime", serde_json::json!({}));
    assert!(err.contains("forbidden"), "got: {err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_container_rejects_invalid_id_shape() {
    let workspace = TempDir::new().unwrap();
    let user_cfg_dir = TempDir::new().unwrap();
    let (app, _registry) = make_app(&[workspace.path()], user_cfg_dir.path()).await;
    let window = make_dashboard_window(&app);

    let err = invoke_err(
        &window,
        "stop_container",
        serde_json::json!({ "containerId": "" }),
    );
    assert!(err.contains("empty"), "got: {err}");

    let huge = "a".repeat(MAX_CONTAINER_ID_BYTES + 1);
    let err = invoke_err(
        &window,
        "stop_container",
        serde_json::json!({ "containerId": huge }),
    );
    assert!(err.contains("too large"), "got: {err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn container_logs_rejects_invalid_id_shape() {
    let workspace = TempDir::new().unwrap();
    let user_cfg_dir = TempDir::new().unwrap();
    let (app, _registry) = make_app(&[workspace.path()], user_cfg_dir.path()).await;
    let window = make_dashboard_window(&app);

    let err = invoke_err(
        &window,
        "container_logs",
        serde_json::json!({ "containerId": "abc;rm -rf" }),
    );
    assert!(err.contains("invalid characters"), "got: {err}");
}
