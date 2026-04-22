//! F-371: authz-rejection tracing.
//!
//! Every `require_window_label*` rejection must emit a `warn!` with target
//! `forge_shell::ipc::authz` and structured `expected`, `actual`, `command`
//! fields. Pins both the negative path of the helpers and the schema, so a
//! rename of any field name breaks the test instead of silently diverging.

#![cfg(feature = "webview")]

mod common;

use forge_shell::ipc::{require_window_label_for_test, require_window_label_in_for_test};

#[test]
fn require_window_label_rejection_emits_warn_with_expected_actual_command() {
    let _g = common::capture_test_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    common::install_capture_subscriber();
    let _ = common::drain_capture();

    let err = require_window_label_for_test(
        "dashboard",   // actual label on the webview
        "session-abc", // required label
        "session_send_message",
    )
    .expect_err("different label must reject");
    assert!(err.contains("forbidden"));

    let logs = common::drain_capture();
    assert!(
        logs.contains("forge_shell::ipc::authz"),
        "expected authz target, got: {logs}"
    );
    assert!(logs.contains("WARN"), "expected WARN level, got: {logs}");
    assert!(
        logs.contains("expected=session-abc"),
        "expected field missing, got: {logs}"
    );
    assert!(
        logs.contains("actual=dashboard"),
        "actual field missing, got: {logs}"
    );
    assert!(
        logs.contains("command=session_send_message"),
        "command field missing, got: {logs}"
    );
}

#[test]
fn require_window_label_in_rejection_emits_warn_with_expected_actual_command() {
    let _g = common::capture_test_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    common::install_capture_subscriber();
    let _ = common::drain_capture();

    let err =
        require_window_label_in_for_test("some-other", &["dashboard"], false, "list_sessions")
            .expect_err("non-listed label must reject");
    assert!(err.contains("forbidden"));

    let logs = common::drain_capture();
    assert!(
        logs.contains("forge_shell::ipc::authz"),
        "expected authz target, got: {logs}"
    );
    assert!(logs.contains("WARN"), "expected WARN level, got: {logs}");
    assert!(
        logs.contains("actual=some-other"),
        "actual field missing, got: {logs}"
    );
    assert!(
        logs.contains("command=list_sessions"),
        "command field missing, got: {logs}"
    );
    assert!(
        logs.contains("dashboard"),
        "expected field should surface the dashboard allow-list, got: {logs}"
    );
}

#[test]
fn require_window_label_success_does_not_log() {
    let _g = common::capture_test_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    common::install_capture_subscriber();
    let _ = common::drain_capture();

    require_window_label_for_test("session-abc", "session-abc", "session_send_message")
        .expect("matching label must pass");

    let logs = common::drain_capture();
    assert!(
        !logs.contains("forge_shell::ipc::authz"),
        "success path must not emit an authz log, got: {logs}"
    );
}
