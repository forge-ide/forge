//! F-051 / H10: per-session IPC authorization.
//!
//! These tests assert that every `#[tauri::command]` handler validates
//! `webview.label()` against the expected owner for its scope:
//! - Session handlers in `ipc.rs` require label `format!("session-{session_id}")`
//! - Dashboard handlers in `dashboard_sessions.rs` require label `"dashboard"`
//!
//! Strategy: build `mock_builder()` apps with `WebviewWindowBuilder` using
//! specific labels, then fire `tauri::test::get_ipc_response` and assert the
//! result shape. For forged-session tests, intentionally leave the *target*
//! session unregistered in `BridgeState`. If authz fires first (as intended)
//! the response is the label-mismatch error; if it silently falls through
//! to the bridge, the response would be a "no active connection" error —
//! which would prove the check is missing.

#![cfg(feature = "webview-test")]

use forge_shell::bridge::SessionConnections;
use forge_shell::ipc::{build_invoke_handler, BridgeState};
use tauri::test::{get_ipc_response, mock_builder, mock_context, noop_assets, INVOKE_KEY};
use tauri::Manager;

const LABEL_MISMATCH: &str = "forbidden: window label mismatch";

fn make_app_with_session_bridge() -> tauri::App<tauri::test::MockRuntime> {
    let app = mock_builder()
        .invoke_handler(build_invoke_handler())
        .build(mock_context(noop_assets()))
        .expect("build mock Tauri app");
    app.manage(BridgeState::new(SessionConnections::new()));
    app
}

fn make_app_with_dashboard_handler() -> tauri::App<tauri::test::MockRuntime> {
    mock_builder()
        .invoke_handler(tauri::generate_handler![
            forge_shell::dashboard_sessions::session_list,
        ])
        .build(mock_context(noop_assets()))
        .expect("build mock Tauri app")
}

fn build_window(
    app: &tauri::App<tauri::test::MockRuntime>,
    label: &str,
) -> tauri::WebviewWindow<tauri::test::MockRuntime> {
    tauri::WebviewWindowBuilder::new(app, label, tauri::WebviewUrl::App("index.html".into()))
        .build()
        .expect("mock window")
}

fn invoke(
    window: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    cmd: &str,
    payload: serde_json::Value,
) -> Result<tauri::ipc::InvokeResponseBody, String> {
    get_ipc_response(
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
    .map_err(|v| match v {
        serde_json::Value::String(s) => s,
        other => other.to_string(),
    })
}

#[test]
fn dashboard_window_invoking_session_approve_tool_is_rejected() {
    let app = make_app_with_session_bridge();
    let window = build_window(&app, "dashboard");

    let err = invoke(
        &window,
        "session_approve_tool",
        serde_json::json!({
            "sessionId": "sess-a",
            "toolCallId": "tc-1",
            "scope": "ThisTool",
        }),
    )
    .expect_err("dashboard window must not call session_approve_tool");

    assert!(
        err.contains(LABEL_MISMATCH),
        "expected label-mismatch error, got: {err}"
    );
}

#[test]
fn session_a_window_invoking_session_approve_tool_for_session_b_is_rejected() {
    // Bridge state has no entries for either session. If the authz check is
    // present the rejection is the label-mismatch error. If it is missing,
    // the bridge would return a "no active connection" error — which would
    // prove the authz layer is absent.
    let app = make_app_with_session_bridge();
    let window = build_window(&app, "session-A");

    let err = invoke(
        &window,
        "session_approve_tool",
        serde_json::json!({
            "sessionId": "B",
            "toolCallId": "tc-forged",
            "scope": "ThisTool",
        }),
    )
    .expect_err("session-A must not approve tool for session B");

    assert!(
        err.contains(LABEL_MISMATCH),
        "expected label-mismatch error (not a bridge error), got: {err}"
    );
}

#[test]
fn session_a_window_invoking_session_send_message_for_session_b_is_rejected() {
    let app = make_app_with_session_bridge();
    let window = build_window(&app, "session-A");

    let err = invoke(
        &window,
        "session_send_message",
        serde_json::json!({
            "sessionId": "B",
            "text": "hello",
        }),
    )
    .expect_err("session-A must not send message for session B");

    assert!(
        err.contains(LABEL_MISMATCH),
        "expected label-mismatch error, got: {err}"
    );
}

#[test]
fn session_a_window_invoking_session_reject_tool_for_session_b_is_rejected() {
    let app = make_app_with_session_bridge();
    let window = build_window(&app, "session-A");

    let err = invoke(
        &window,
        "session_reject_tool",
        serde_json::json!({
            "sessionId": "B",
            "toolCallId": "tc-forged",
            "reason": null,
        }),
    )
    .expect_err("session-A must not reject tool for session B");

    assert!(
        err.contains(LABEL_MISMATCH),
        "expected label-mismatch error, got: {err}"
    );
}

#[test]
fn session_a_window_invoking_session_subscribe_for_session_b_is_rejected() {
    let app = make_app_with_session_bridge();
    let window = build_window(&app, "session-A");

    let err = invoke(
        &window,
        "session_subscribe",
        serde_json::json!({
            "sessionId": "B",
            "since": 0,
        }),
    )
    .expect_err("session-A must not subscribe to session B");

    assert!(
        err.contains(LABEL_MISMATCH),
        "expected label-mismatch error, got: {err}"
    );
}

#[test]
fn session_a_window_invoking_session_hello_for_session_b_is_rejected() {
    let app = make_app_with_session_bridge();
    let window = build_window(&app, "session-A");

    let err = invoke(
        &window,
        "session_hello",
        serde_json::json!({
            "sessionId": "B",
        }),
    )
    .expect_err("session-A must not perform hello for session B");

    assert!(
        err.contains(LABEL_MISMATCH),
        "expected label-mismatch error, got: {err}"
    );
}

#[test]
fn non_dashboard_window_invoking_session_list_is_rejected() {
    let app = make_app_with_dashboard_handler();
    let window = build_window(&app, "session-A");

    let err = invoke(&window, "session_list", serde_json::json!({}))
        .expect_err("non-dashboard window must not call session_list");

    assert!(
        err.contains(LABEL_MISMATCH),
        "expected label-mismatch error, got: {err}"
    );
}

#[test]
fn non_dashboard_window_invoking_open_session_is_rejected() {
    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![
            forge_shell::dashboard_sessions::open_session,
        ])
        .build(mock_context(noop_assets()))
        .expect("build mock Tauri app");
    let window = build_window(&app, "session-A");

    let err = invoke(
        &window,
        "open_session",
        serde_json::json!({ "id": "some-session-id" }),
    )
    .expect_err("non-dashboard window must not call open_session");

    assert!(
        err.contains(LABEL_MISMATCH),
        "expected label-mismatch error, got: {err}"
    );
}
