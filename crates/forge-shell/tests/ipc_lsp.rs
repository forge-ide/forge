//! F-123 IPC integration tests for the `lsp_*` Tauri commands.
//!
//! Exercises the full invoke → `forge-lsp::Server` spawn → message-event
//! emit path via `tauri::test::get_ipc_response`. Mirrors the F-125
//! `ipc_terminal.rs` shape:
//!
//! - authz: dashboard is rejected.
//! - spawn binds the server to the calling webview's label.
//! - cross-session send/stop is rejected with label-mismatch.
//! - a round-trip via the stub LSP fixture proves the message event reaches
//!   the owner webview (and only the owner).
//!
//! We reuse the `forge-lsp-mock-stdio` fixture from the forge-lsp crate —
//! `env!("CARGO_BIN_EXE_forge-lsp-mock-stdio")` resolves to the test-time
//! fixture binary path.

#![cfg(feature = "webview-test")]

use std::time::{Duration, Instant};

use forge_shell::ipc::{build_invoke_handler, manage_lsp};
use serde_json::Value;
use tauri::test::{get_ipc_response, mock_builder, mock_context, noop_assets, INVOKE_KEY};
use tauri::Listener;

const LABEL_MISMATCH: &str = "forbidden: window label mismatch";

fn mock_stdio_path() -> String {
    // Depend on the fixture binary built by the `forge-lsp` crate. Cargo
    // does not export sibling crate bin paths via `CARGO_BIN_EXE_*`, so
    // we resolve the target dir explicitly.
    let exe = std::env::current_exe().expect("current_exe");
    // target/debug/deps/ipc_lsp-HASH → target/debug/
    let mut dir = exe
        .parent()
        .and_then(|p| p.parent())
        .expect("target/debug dir")
        .to_path_buf();
    dir.push("forge-lsp-mock-stdio");
    if cfg!(windows) {
        dir.set_extension("exe");
    }
    assert!(
        dir.exists(),
        "fixture must be built; run `cargo build -p forge-lsp --bin forge-lsp-mock-stdio`. \
         missing: {}",
        dir.display()
    );
    dir.to_string_lossy().into_owned()
}

fn make_app() -> tauri::App<tauri::test::MockRuntime> {
    let app = mock_builder()
        .invoke_handler(build_invoke_handler())
        .build(mock_context(noop_assets()))
        .expect("build mock Tauri app");
    manage_lsp(&app.handle().clone());
    app
}

fn make_window(
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

// ---------------------------------------------------------------------------
// Authorization
// ---------------------------------------------------------------------------

#[test]
fn dashboard_window_cannot_start_an_lsp_server() {
    let app = make_app();
    let window = make_window(&app, "dashboard");
    let err = invoke(
        &window,
        "lsp_start",
        serde_json::json!({
            "args": {
                "server": "rust-analyzer",
                "binary_path": mock_stdio_path(),
                "args": [],
            }
        }),
    )
    .expect_err("dashboard window must not start an lsp server");

    assert!(
        err.contains(LABEL_MISMATCH),
        "expected label-mismatch error, got: {err}"
    );
}

#[test]
fn cross_session_send_is_rejected_with_label_mismatch() {
    // Alice starts an LSP server; Bob tries to send to it. The registry
    // binds the server to alice's label and must reject bob's invoke.
    let app = make_app();
    let alice = make_window(&app, "session-alice-lsp");
    let bob = make_window(&app, "session-bob-lsp");

    invoke(
        &alice,
        "lsp_start",
        serde_json::json!({
            "args": {
                "server": "alice-srv",
                "binary_path": mock_stdio_path(),
                "args": [],
            }
        }),
    )
    .expect("alice start");

    let err = invoke(
        &bob,
        "lsp_send",
        serde_json::json!({
            "server": "alice-srv",
            "message": {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}
        }),
    )
    .expect_err("bob must not send to alice's server");
    assert!(
        err.contains(LABEL_MISMATCH),
        "expected label-mismatch error for send, got: {err}"
    );

    let err = invoke(&bob, "lsp_stop", serde_json::json!({"server": "alice-srv"}))
        .expect_err("bob must not stop alice's server");
    assert!(
        err.contains(LABEL_MISMATCH),
        "expected label-mismatch error for stop, got: {err}"
    );

    // Clean up via the owner.
    invoke(
        &alice,
        "lsp_stop",
        serde_json::json!({"server": "alice-srv"}),
    )
    .expect("owner stop");
}

// ---------------------------------------------------------------------------
// DoD: initialize → lsp_message event → shutdown round trip via IPC
// ---------------------------------------------------------------------------

/// Collect `lsp_message` payloads targeted at `window` until `deadline`
/// elapses. Returns parsed messages so assertions can inspect structure.
fn drain_lsp_messages(
    window: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    deadline: Instant,
) -> Vec<Value> {
    use std::sync::{Arc, Mutex};
    let collected: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&collected);
    let _listener = window.listen("lsp_message", move |ev| {
        if let Ok(v) = serde_json::from_str::<Value>(ev.payload()) {
            sink.lock().unwrap().push(v);
        }
    });
    while Instant::now() < deadline {
        if !collected.lock().unwrap().is_empty() {
            // Don't break on first event — a couple of frames may arrive
            // close together; give the reader a beat.
            std::thread::sleep(Duration::from_millis(50));
            if collected.lock().unwrap().len() >= 2 {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let out = collected.lock().unwrap().clone();
    out
}

#[test]
fn initialize_round_trip_emits_lsp_message_on_owner_webview() {
    let app = make_app();
    let window = make_window(&app, "session-lsp-round");

    invoke(
        &window,
        "lsp_start",
        serde_json::json!({
            "args": {
                "server": "mock-srv",
                "binary_path": mock_stdio_path(),
                "args": [],
            }
        }),
    )
    .expect("lsp_start");

    // Give the supervisor a moment to spawn the child + install stdin. The
    // transport returns `server not running` until then; retrying until
    // success mirrors the pattern in the forge-lsp integration test.
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let attempt = invoke(
            &window,
            "lsp_send",
            serde_json::json!({
                "server": "mock-srv",
                "message": {
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "initialize",
                    "params": {}
                }
            }),
        );
        if attempt.is_ok() {
            break;
        }
        if Instant::now() >= deadline {
            panic!("lsp_send never succeeded: {attempt:?}");
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    // Collect the response event.
    let messages = drain_lsp_messages(&window, Instant::now() + Duration::from_secs(5));
    assert!(
        !messages.is_empty(),
        "expected at least one lsp_message event"
    );

    // Verify shape: { server: "mock-srv", message: { id, result.capabilities } }
    let first = &messages[0];
    assert_eq!(
        first.get("server").and_then(|v| v.as_str()),
        Some("mock-srv"),
        "payload.server must match start arg; got {first}"
    );
    let msg = first.get("message").expect("payload.message present");
    assert_eq!(
        msg.get("id").and_then(|v| v.as_u64()),
        Some(1),
        "response id must match request"
    );
    assert!(
        msg.pointer("/result/capabilities").is_some(),
        "initialize result.capabilities missing: {msg}"
    );

    invoke(
        &window,
        "lsp_stop",
        serde_json::json!({"server": "mock-srv"}),
    )
    .expect("lsp_stop");
}

#[test]
fn duplicate_server_id_is_rejected() {
    let app = make_app();
    let window = make_window(&app, "session-lsp-dup");

    invoke(
        &window,
        "lsp_start",
        serde_json::json!({
            "args": {
                "server": "dup-srv",
                "binary_path": mock_stdio_path(),
                "args": [],
            }
        }),
    )
    .expect("first start");

    let err = invoke(
        &window,
        "lsp_start",
        serde_json::json!({
            "args": {
                "server": "dup-srv",
                "binary_path": mock_stdio_path(),
                "args": [],
            }
        }),
    )
    .expect_err("duplicate id must be rejected");
    assert!(
        err.contains("already running"),
        "expected duplicate-id error, got: {err}"
    );

    invoke(
        &window,
        "lsp_stop",
        serde_json::json!({"server": "dup-srv"}),
    )
    .expect("stop");
}

#[test]
fn oversize_message_is_rejected_at_command_layer() {
    // A 1 MiB payload exceeds MAX_LSP_MESSAGE_BYTES (512 KiB). The command
    // must reject before the transport touches stdin.
    let app = make_app();
    let window = make_window(&app, "session-lsp-cap");

    invoke(
        &window,
        "lsp_start",
        serde_json::json!({
            "args": {
                "server": "cap-srv",
                "binary_path": mock_stdio_path(),
                "args": [],
            }
        }),
    )
    .expect("start");

    let huge = "A".repeat(1024 * 1024);
    let err = invoke(
        &window,
        "lsp_send",
        serde_json::json!({
            "server": "cap-srv",
            "message": {"blob": huge}
        }),
    )
    .expect_err("oversize must be rejected");
    assert!(
        err.contains("payload too large") && err.contains("message"),
        "expected message size-cap error, got: {err}"
    );

    invoke(
        &window,
        "lsp_stop",
        serde_json::json!({"server": "cap-srv"}),
    )
    .expect("stop");
}
