//! F-122 filesystem Tauri command tests.
//!
//! Covers the three new commands — `read_file`, `write_file`, `tree` —
//! through Tauri's `test::get_ipc_response` machinery. Window-label authz
//! and `forge-fs` path-denial paths are both exercised here so a regression
//! on either layer fails the test rather than widening the sandbox.

#![cfg(feature = "webview-test")]

use std::fs;

use forge_shell::bridge::SessionConnections;
use forge_shell::ipc::{build_invoke_handler, BridgeState};
use tauri::test::{get_ipc_response, mock_builder, mock_context, noop_assets, INVOKE_KEY};
use tauri::Manager;
use tempfile::TempDir;

const LABEL_MISMATCH: &str = "forbidden: window label mismatch";

fn make_app() -> tauri::App<tauri::test::MockRuntime> {
    let app = mock_builder()
        .invoke_handler(build_invoke_handler())
        .build(mock_context(noop_assets()))
        .expect("build mock Tauri app");
    app.manage(BridgeState::new(SessionConnections::new()));
    app
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
    let res = get_ipc_response(
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
    let res = get_ipc_response(
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
async fn read_file_returns_content_for_a_workspace_file() {
    let workspace = TempDir::new().unwrap();
    let canonical_ws = fs::canonicalize(workspace.path()).unwrap();
    let file = canonical_ws.join("hello.txt");
    fs::write(&file, "hello\n").unwrap();

    let app = make_app();
    let window = make_session_window(&app, "abcdef0123456789");

    let result = invoke_ok(
        &window,
        "read_file",
        serde_json::json!({
            "sessionId": "abcdef0123456789",
            "workspaceRoot": canonical_ws,
            "path": file,
        }),
    );
    assert_eq!(result["content"], serde_json::json!("hello\n"));
    assert_eq!(result["bytes"], serde_json::json!(6));
}

#[tokio::test(flavor = "multi_thread")]
async fn read_file_rejects_paths_outside_the_workspace() {
    let workspace = TempDir::new().unwrap();
    let canonical_ws = fs::canonicalize(workspace.path()).unwrap();
    let outside = TempDir::new().unwrap();
    let canonical_outside = fs::canonicalize(outside.path()).unwrap();
    let victim = canonical_outside.join("secret.txt");
    fs::write(&victim, "do not leak").unwrap();

    let app = make_app();
    let window = make_session_window(&app, "abcdef0123456789");

    let err = invoke_err(
        &window,
        "read_file",
        serde_json::json!({
            "sessionId": "abcdef0123456789",
            "workspaceRoot": canonical_ws,
            "path": victim,
        }),
    );
    // forge-fs emits "path '…' is not allowed by allowed_paths".
    assert!(
        err.contains("not allowed") || err.contains("PathDenied"),
        "expected path-denied error, got: {err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn read_file_rejects_a_dashboard_window() {
    let workspace = TempDir::new().unwrap();
    let canonical_ws = fs::canonicalize(workspace.path()).unwrap();
    let file = canonical_ws.join("hello.txt");
    fs::write(&file, "hi").unwrap();

    let app = make_app();
    let window = make_dashboard_window(&app);

    let err = invoke_err(
        &window,
        "read_file",
        serde_json::json!({
            "sessionId": "abcdef0123456789",
            "workspaceRoot": canonical_ws,
            "path": file,
        }),
    );
    assert!(
        err.contains(LABEL_MISMATCH),
        "dashboard window must be rejected, got: {err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn write_file_creates_a_new_file_and_roundtrips() {
    let workspace = TempDir::new().unwrap();
    let canonical_ws = fs::canonicalize(workspace.path()).unwrap();
    let file = canonical_ws.join("new.txt");
    let payload = "created by write_file\n";
    let bytes_payload: Vec<u8> = payload.as_bytes().to_vec();

    let app = make_app();
    let window = make_session_window(&app, "abcdef0123456789");

    invoke_ok(
        &window,
        "write_file",
        serde_json::json!({
            "sessionId": "abcdef0123456789",
            "workspaceRoot": canonical_ws,
            "path": file,
            "bytes": bytes_payload,
        }),
    );

    let on_disk = fs::read_to_string(&file).expect("file must exist after write");
    assert_eq!(on_disk, payload);
}

#[tokio::test(flavor = "multi_thread")]
async fn write_file_rejects_paths_outside_the_workspace() {
    let workspace = TempDir::new().unwrap();
    let canonical_ws = fs::canonicalize(workspace.path()).unwrap();
    let outside = TempDir::new().unwrap();
    let canonical_outside = fs::canonicalize(outside.path()).unwrap();
    let victim = canonical_outside.join("attacker-written.txt");

    let app = make_app();
    let window = make_session_window(&app, "abcdef0123456789");

    let err = invoke_err(
        &window,
        "write_file",
        serde_json::json!({
            "sessionId": "abcdef0123456789",
            "workspaceRoot": canonical_ws,
            "path": victim,
            "bytes": b"x".to_vec(),
        }),
    );
    assert!(
        err.contains("not allowed") || err.contains("PathDenied"),
        "expected path-denied error, got: {err}"
    );
    assert!(
        !victim.exists(),
        "path-denied write must not create a file: {victim:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn write_file_rejects_a_non_session_window() {
    let workspace = TempDir::new().unwrap();
    let canonical_ws = fs::canonicalize(workspace.path()).unwrap();
    let file = canonical_ws.join("forbidden.txt");

    let app = make_app();
    let window = make_dashboard_window(&app);

    let err = invoke_err(
        &window,
        "write_file",
        serde_json::json!({
            "sessionId": "abcdef0123456789",
            "workspaceRoot": canonical_ws,
            "path": file,
            "bytes": b"nope".to_vec(),
        }),
    );
    assert!(
        err.contains(LABEL_MISMATCH),
        "dashboard must not invoke write_file, got: {err}"
    );
    assert!(!file.exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn tree_lists_files_inside_the_workspace() {
    let workspace = TempDir::new().unwrap();
    let canonical_ws = fs::canonicalize(workspace.path()).unwrap();
    fs::write(canonical_ws.join("a.txt"), "a").unwrap();
    fs::create_dir(canonical_ws.join("sub")).unwrap();
    fs::write(canonical_ws.join("sub/b.txt"), "b").unwrap();

    let app = make_app();
    let window = make_session_window(&app, "abcdef0123456789");

    let result = invoke_ok(
        &window,
        "tree",
        serde_json::json!({
            "sessionId": "abcdef0123456789",
            "workspaceRoot": canonical_ws,
            "root": canonical_ws,
            "depth": 4,
        }),
    );
    assert_eq!(result["kind"], serde_json::json!("Dir"));
    let children = result["children"].as_array().expect("children array");
    let names: Vec<_> = children
        .iter()
        .map(|n| n["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["a.txt", "sub"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn tree_rejects_roots_outside_the_workspace() {
    let workspace = TempDir::new().unwrap();
    let canonical_ws = fs::canonicalize(workspace.path()).unwrap();
    let outside = TempDir::new().unwrap();
    let canonical_outside = fs::canonicalize(outside.path()).unwrap();

    let app = make_app();
    let window = make_session_window(&app, "abcdef0123456789");

    let err = invoke_err(
        &window,
        "tree",
        serde_json::json!({
            "sessionId": "abcdef0123456789",
            "workspaceRoot": canonical_ws,
            "root": canonical_outside,
            "depth": 4,
        }),
    );
    assert!(
        err.contains("not allowed") || err.contains("PathDenied"),
        "expected path-denied error for out-of-workspace root, got: {err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tree_rejects_mismatched_session_label() {
    let workspace = TempDir::new().unwrap();
    let canonical_ws = fs::canonicalize(workspace.path()).unwrap();

    let app = make_app();
    // Window label is `session-A`, but the invoke claims session `B`.
    let window = make_session_window(&app, "A");

    let err = invoke_err(
        &window,
        "tree",
        serde_json::json!({
            "sessionId": "B",
            "workspaceRoot": canonical_ws,
            "root": canonical_ws,
            "depth": 4,
        }),
    );
    assert!(
        err.contains(LABEL_MISMATCH),
        "cross-session invoke must be rejected, got: {err}"
    );
}
