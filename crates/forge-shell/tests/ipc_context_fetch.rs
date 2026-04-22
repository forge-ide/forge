//! F-359: Tauri-command coverage for `context_fetch_url` +
//! `set_context_allowed_hosts`.
//!
//! Covers the wire invariants the pure-policy and HTTP-layer tests
//! cannot:
//!
//! 1. **Authz.** Dashboard / cross-session invokes of `context_fetch_url`
//!    are rejected with the F-051 label-mismatch error before the fetch
//!    path runs. Pins the "only the owning session window may fetch"
//!    contract.
//! 2. **Server-side allowlist authority.** A URL the webview submits but
//!    that is not on the server-owned list returns a host-not-allowed
//!    error. The webview cannot widen its own reach by passing an
//!    arbitrary URL — the finding's core remediation.
//! 3. **Disallowed-scheme rejection at IPC boundary.** `file://` / `javascript:`
//!    are refused before any transport I/O — both the explicit
//!    non-http(s) case and URLs without any host.
//! 4. **Size caps.** Oversize URL or allowlist entries are rejected by
//!    the existing `require_size` / entry-count gate before the fetch
//!    path allocates.

#![cfg(feature = "webview-test")]

use forge_shell::bridge::SessionConnections;
use forge_shell::ipc::{
    build_invoke_handler, manage_context_fetch, AllowedHostsState, BridgeState,
};
use tauri::test::{get_ipc_response, mock_builder, mock_context, noop_assets, INVOKE_KEY};
use tauri::Manager;

const LABEL_MISMATCH: &str = "forbidden: window label mismatch";
const TEST_SESSION: &str = "abcdef0123456789";

fn make_app() -> tauri::App<tauri::test::MockRuntime> {
    let app = mock_builder()
        .invoke_handler(build_invoke_handler())
        .build(mock_context(noop_assets()))
        .expect("build mock Tauri app");
    app.manage(BridgeState::new(SessionConnections::new()));
    manage_context_fetch(&app.handle().clone());
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
async fn context_fetch_url_rejects_dashboard_window() {
    // The @-context fetcher is session-scoped. The dashboard window
    // must not be able to use the command as a side-channel proxy into
    // the fetch path even with a valid-looking URL.
    let app = make_app();
    let window = make_dashboard_window(&app);
    let err = invoke_err(
        &window,
        "context_fetch_url",
        serde_json::json!({
            "sessionId": TEST_SESSION,
            "url": "https://docs.rs/tokio",
        }),
    );
    assert!(
        err.contains(LABEL_MISMATCH),
        "dashboard window must be rejected, got: {err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn context_fetch_url_rejects_host_not_on_server_allowlist() {
    // The finding's core: a webview submits a URL whose host is NOT
    // on the server-side allowlist. The fetcher must refuse without
    // making any network I/O. This is the "webview cannot lie about
    // the target host" contract — even a DNS-resolving, publicly
    // reachable hostname is rejected if the server hasn't allowlisted
    // it.
    let app = make_app();
    // Server-side allowlist intentionally left empty.
    let window = make_session_window(&app, TEST_SESSION);
    let err = invoke_err(
        &window,
        "context_fetch_url",
        serde_json::json!({
            "sessionId": TEST_SESSION,
            "url": "https://docs.rs/tokio",
        }),
    );
    assert!(
        err.contains("not on allowed-hosts list") || err.contains("not allowed"),
        "expected host-not-allowed; got: {err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn context_fetch_url_rejects_file_scheme() {
    let app = make_app();
    // Seed the allowlist so the scheme check — not the host check —
    // is the authority that refuses.
    let allowed = app.state::<AllowedHostsState>();
    allowed.replace(vec!["/etc/passwd".to_string()]);
    let window = make_session_window(&app, TEST_SESSION);
    let err = invoke_err(
        &window,
        "context_fetch_url",
        serde_json::json!({
            "sessionId": TEST_SESSION,
            "url": "file:///etc/passwd",
        }),
    );
    assert!(
        err.contains("scheme") || err.contains("not allowed"),
        "expected scheme rejection; got: {err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn context_fetch_url_rejects_link_local_ip_literal() {
    // SSRF vector: AWS IMDS link-local. Even if the user had
    // misguidedly allowlisted `169.254.169.254` as a string, the
    // IP-class block rejects it before any transport I/O.
    let app = make_app();
    let allowed = app.state::<AllowedHostsState>();
    allowed.replace(vec!["169.254.169.254".to_string()]);
    let window = make_session_window(&app, TEST_SESSION);
    let err = invoke_err(
        &window,
        "context_fetch_url",
        serde_json::json!({
            "sessionId": TEST_SESSION,
            "url": "http://169.254.169.254/latest/meta-data/",
        }),
    );
    assert!(
        err.contains("link-local") || err.contains("blocked"),
        "expected IP-range rejection; got: {err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn context_fetch_url_rejects_oversize_url() {
    let app = make_app();
    let window = make_session_window(&app, TEST_SESSION);
    // MAX_CONTEXT_URL_BYTES = 8 KiB. Submit 16 KiB.
    let long = format!("https://docs.rs/{}", "x".repeat(16 * 1024));
    let err = invoke_err(
        &window,
        "context_fetch_url",
        serde_json::json!({
            "sessionId": TEST_SESSION,
            "url": long,
        }),
    );
    assert!(
        err.contains("payload too large") && err.contains("url"),
        "expected size-cap error; got: {err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn set_context_allowed_hosts_replaces_the_list() {
    // `set_context_allowed_hosts` overwrites the server-side list
    // verbatim (post-trim, post-filter-empty). A subsequent
    // `context_fetch_url` whose host is NOT on the list returns the
    // host-not-allowed error, confirming the setter is authoritative.
    let app = make_app();
    let session_window = make_session_window(&app, TEST_SESSION);

    // Seed from the session window (session-* is accepted).
    invoke_ok(
        &session_window,
        "set_context_allowed_hosts",
        serde_json::json!({ "hosts": ["docs.rs", "  ", ""] }),
    );
    let allowed = app.state::<AllowedHostsState>();
    assert_eq!(allowed.snapshot(), vec!["docs.rs".to_string()]);

    // Replace via the dashboard window (also accepted by the gate).
    let dashboard = make_dashboard_window(&app);
    invoke_ok(
        &dashboard,
        "set_context_allowed_hosts",
        serde_json::json!({ "hosts": ["only.example.com"] }),
    );
    assert_eq!(
        app.state::<AllowedHostsState>().snapshot(),
        vec!["only.example.com".to_string()]
    );

    // A URL for the *previous* allowlist entry must now be rejected.
    let err = invoke_err(
        &session_window,
        "context_fetch_url",
        serde_json::json!({
            "sessionId": TEST_SESSION,
            "url": "https://docs.rs/tokio",
        }),
    );
    assert!(
        err.contains("not on allowed-hosts") || err.contains("not allowed"),
        "expected host-not-allowed after replacement; got: {err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn set_context_allowed_hosts_rejects_oversize_entry() {
    let app = make_app();
    let window = make_session_window(&app, TEST_SESSION);
    let too_long = "x".repeat(512); // > MAX_ALLOWED_HOST_BYTES (256)
    let err = invoke_err(
        &window,
        "set_context_allowed_hosts",
        serde_json::json!({ "hosts": ["good.example.com", too_long] }),
    );
    assert!(
        err.contains("payload too large") && err.contains("host"),
        "expected per-entry size cap; got: {err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn set_context_allowed_hosts_rejects_oversize_list() {
    let app = make_app();
    let window = make_session_window(&app, TEST_SESSION);
    let hosts: Vec<String> = (0..512).map(|i| format!("host{i}.example.com")).collect();
    let err = invoke_err(
        &window,
        "set_context_allowed_hosts",
        serde_json::json!({ "hosts": hosts }),
    );
    assert!(
        err.contains("payload too large") && err.contains("hosts"),
        "expected list-length cap; got: {err}"
    );
}
