//! RED 3: The Tauri `ipc` module must expose the five commands required by
//! F-020 and register them with `tauri::generate_handler!` without errors.
//!
//! These tests run under the `webview` feature so they can invoke real
//! Tauri machinery via `tauri::test::mock_builder`.

#![cfg(feature = "webview-test")]

use std::sync::Arc;

use forge_providers::MockProvider;
use forge_session::server::serve_with_session;
use forge_session::session::Session;
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

#[test]
fn invoke_handler_builds_without_error() {
    let app = make_app();
    // Attach fresh bridge state so commands have something to resolve against.
    app.manage(BridgeState::new(SessionConnections::new()));
    // Nothing else — this test just proves `build_invoke_handler()` and the
    // five `#[tauri::command]` handlers compile and register together.
    drop(app);
}

#[tokio::test(flavor = "multi_thread")]
async fn session_hello_command_round_trips_via_tauri_invoke() {
    let sock_dir = TempDir::new().unwrap();
    let sock = sock_dir.path().join("tauri-hello.sock");

    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("events.jsonl");
    let session = Arc::new(Session::create(log_path).await.unwrap());
    let provider = Arc::new(MockProvider::with_default_path());
    let server_sock = sock.clone();
    tokio::spawn(async move {
        serve_with_session(
            &server_sock,
            session,
            provider,
            true,
            false,
            None,
            Some("tauri-hello".to_string()),
        )
        .await
        .unwrap();
    });
    for _ in 0..50 {
        if sock.exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    let app = make_app();
    app.manage(BridgeState::new(SessionConnections::new()));

    // F-051: window label must match `session-{session_id}` for the
    // session_hello authz check to pass.
    let window = tauri::WebviewWindowBuilder::new(
        &app,
        "session-tauri-hello",
        tauri::WebviewUrl::App("index.html".into()),
    )
    .build()
    .expect("mock window");

    let payload = serde_json::json!({
        "sessionId": "tauri-hello",
        "socketPath": sock.to_string_lossy(),
    });
    let res = tauri::test::get_ipc_response(
        &window,
        tauri::webview::InvokeRequest {
            cmd: "session_hello".into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: "http://tauri.localhost".parse().unwrap(),
            body: tauri::ipc::InvokeBody::Json(payload),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        },
    );

    let value = res.expect("session_hello returns HelloAck JSON");
    let obj = value.deserialize::<serde_json::Value>().unwrap();
    assert_eq!(obj["session_id"], "tauri-hello");
    assert_eq!(obj["schema_version"], 1);
}
