//! Tauri `ipc` command surface tests for F-020 and F-052.
//!
//! F-020: `build_invoke_handler()` and the five `#[tauri::command]` handlers
//! compile and register together; `session_hello` round-trips against a real
//! daemon via `tauri::test::get_ipc_response`.
//!
//! F-052 (H11 / T7): the production `session_hello` command does **not**
//! accept an arbitrary `socketPath` — any value passed by a webview caller is
//! ignored. The round-trip test uses a `webview-test`-gated
//! [`BridgeState::with_test_socket_override`] constructor instead of a
//! public parameter; the regression test spins up a rogue listener and
//! asserts it never receives a connection even when its path is injected
//! via the `socketPath` JSON field.

#![cfg(feature = "webview-test")]

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use forge_providers::MockProvider;
use forge_session::server::serve_with_session;
use forge_session::session::Session;
use forge_shell::bridge::SessionConnections;
use forge_shell::ipc::{build_invoke_handler, BridgeState};
use tauri::test::{mock_builder, mock_context, noop_assets, INVOKE_KEY};
use tauri::Manager;
use tempfile::TempDir;
use tokio::net::UnixListener;

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
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let app = make_app();
    // F-052: production `session_hello` no longer accepts a `socketPath`
    // parameter — the test daemon's path is wired through a test-only
    // constructor on `BridgeState` gated behind the `webview-test` feature.
    app.manage(BridgeState::with_test_socket_override(
        SessionConnections::new(),
        sock.clone(),
    ));

    // F-051: window label must match `session-{session_id}` for the
    // session_hello authz check to pass.
    let window = tauri::WebviewWindowBuilder::new(
        &app,
        "session-tauri-hello",
        tauri::WebviewUrl::App("index.html".into()),
    )
    .build()
    .expect("mock window");

    let payload = serde_json::json!({ "sessionId": "tauri-hello" });
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

/// F-052 regression: a webview that injects an attacker-controlled
/// `socketPath` into the `session_hello` invoke payload must not cause the
/// shell to connect to that path. We bind a rogue `UnixListener` at a
/// tempdir path, invoke `session_hello` with `socketPath` set to that path,
/// and assert the rogue listener never accepts a connection. The command
/// should instead attempt the default path (which has no daemon) and fail.
#[tokio::test(flavor = "multi_thread")]
async fn session_hello_ignores_attacker_supplied_socket_path() {
    let rogue_dir = TempDir::new().unwrap();
    let rogue_sock = rogue_dir.path().join("attacker.sock");
    let rogue_listener = UnixListener::bind(&rogue_sock).expect("bind rogue UDS");

    let accept_count = Arc::new(AtomicU32::new(0));
    let accept_count_bg = Arc::clone(&accept_count);
    let accept_task = tokio::spawn(async move {
        // If the shell ever connects here, record it. The test must see 0.
        if let Ok((_stream, _addr)) = rogue_listener.accept().await {
            accept_count_bg.fetch_add(1, Ordering::SeqCst);
        }
    });

    let app = make_app();
    // Production path: no test override → default_socket_path will be used.
    app.manage(BridgeState::new(SessionConnections::new()));

    let window = tauri::WebviewWindowBuilder::new(
        &app,
        "session-attacker",
        tauri::WebviewUrl::App("index.html".into()),
    )
    .build()
    .expect("mock window");

    let payload = serde_json::json!({
        "sessionId": "attacker",
        // Injected by a webview caller. With F-052 this field is unknown
        // to the command and silently ignored by serde.
        "socketPath": rogue_sock.to_string_lossy(),
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

    // No daemon at the default path → the call fails. The security-meaningful
    // assertion is that the rogue listener never accepted.
    assert!(
        res.is_err(),
        "session_hello must not succeed when no daemon is listening at the default path"
    );

    // Give the accept task a brief window to (not) fire.
    tokio::time::sleep(Duration::from_millis(100)).await;
    accept_task.abort();
    let _ = accept_task.await;

    assert_eq!(
        accept_count.load(Ordering::SeqCst),
        0,
        "rogue listener must not receive any connection from session_hello"
    );
}
