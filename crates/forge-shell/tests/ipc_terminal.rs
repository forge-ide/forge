//! F-125 IPC integration tests for the `terminal_*` Tauri commands.
//!
//! These tests exercise the full invoke → `forge-term` spawn → byte-event
//! emit path via `tauri::test::get_ipc_response`. They:
//!
//! - verify authz (dashboard / wrong-session windows are rejected),
//! - prove spawn registers the terminal against the calling webview's label,
//! - prove write reaches the child (round-trips through a `sh -c "read X;
//!   printf GOT=%s $X"` shell),
//! - prove resize updates PTY dimensions (checked via `stty size`),
//! - prove kill tears the child down and the second kill is rejected.
//!
//! Cross-session isolation: a separate test spawns a terminal from
//! `session-alice`, then tries to write/resize/kill it from
//! `session-bob` and asserts every call is rejected with the label-mismatch
//! error (not a bridge-level "no active connection"-style fall-through).

#![cfg(feature = "webview-test")]

use std::time::{Duration, Instant};

use forge_shell::ipc::{build_invoke_handler, manage_terminals};
use serde_json::Value;
use tauri::test::{get_ipc_response, mock_builder, mock_context, noop_assets, INVOKE_KEY};
use tauri::Listener;

const LABEL_MISMATCH: &str = "forbidden: window label mismatch";

fn make_app() -> tauri::App<tauri::test::MockRuntime> {
    let app = mock_builder()
        .invoke_handler(build_invoke_handler())
        .build(mock_context(noop_assets()))
        .expect("build mock Tauri app");
    manage_terminals(&app.handle().clone());
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

fn fresh_terminal_id() -> String {
    // The Rust-side `TerminalId` uses `Arc<str>` hex from 8 random bytes.
    // We mint one here with the same shape so the wire payload is valid.
    use std::time::SystemTime;
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    // 16 lowercase hex chars; `nanos` is globally monotonic inside a run so
    // id-reuse collisions between tests don't happen.
    format!(
        "{:016x}",
        (nanos as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
    )
}

// ---------------------------------------------------------------------------
// Authorization
// ---------------------------------------------------------------------------

#[test]
fn dashboard_window_cannot_spawn_a_terminal() {
    let app = make_app();
    let window = make_window(&app, "dashboard");

    let err = invoke(
        &window,
        "terminal_spawn",
        serde_json::json!({
            "args": {
                "terminal_id": fresh_terminal_id(),
                "shell": null,
                "cwd": "/tmp",
                "cols": 80,
                "rows": 24,
            }
        }),
    )
    .expect_err("dashboard window must not spawn terminals");

    assert!(
        err.contains(LABEL_MISMATCH),
        "expected label-mismatch error, got: {err}"
    );
}

#[test]
fn cross_session_write_is_rejected_with_label_mismatch() {
    // Alice spawns; Bob tries to write with the same id. The registry
    // binds the terminal to alice's label and must reject bob's invoke
    // with LABEL_MISMATCH — not a "terminal gone"-style bridge fall-through.
    let app = make_app();
    let alice = make_window(&app, "session-alice");
    let bob = make_window(&app, "session-bob");

    let tid = fresh_terminal_id();

    invoke(
        &alice,
        "terminal_spawn",
        serde_json::json!({
            "args": {
                "terminal_id": tid.clone(),
                "shell": null,
                "cwd": std::env::temp_dir().to_string_lossy(),
                "cols": 80,
                "rows": 24,
            }
        }),
    )
    .expect("alice spawn must succeed");

    // Bob forges a write. The terminal_id exists, but the owner_label
    // doesn't match bob's label → LABEL_MISMATCH before any PTY I/O.
    let err = invoke(
        &bob,
        "terminal_write",
        serde_json::json!({ "terminalId": tid.clone(), "data": vec![b'x'] }),
    )
    .expect_err("bob must not write to alice's terminal");
    assert!(
        err.contains(LABEL_MISMATCH),
        "expected label-mismatch error, got: {err}"
    );

    // Ditto for resize.
    let err = invoke(
        &bob,
        "terminal_resize",
        serde_json::json!({ "terminalId": tid.clone(), "cols": 132u16, "rows": 50u16 }),
    )
    .expect_err("bob must not resize alice's terminal");
    assert!(
        err.contains(LABEL_MISMATCH),
        "expected label-mismatch error, got: {err}"
    );

    // And for kill.
    let err = invoke(
        &bob,
        "terminal_kill",
        serde_json::json!({ "terminalId": tid.clone() }),
    )
    .expect_err("bob must not kill alice's terminal");
    assert!(
        err.contains(LABEL_MISMATCH),
        "expected label-mismatch error, got: {err}"
    );

    // Cleanup — alice owns the terminal so her kill must succeed.
    invoke(
        &alice,
        "terminal_kill",
        serde_json::json!({ "terminalId": tid }),
    )
    .expect("owner kill succeeds");
}

// ---------------------------------------------------------------------------
// DoD: spawn + echo + resize + kill round trip
// ---------------------------------------------------------------------------

/// Collect `terminal:bytes` payloads targeted at the given window until
/// `deadline` elapses. Uses Tauri's `listen_any` on the window so the stream
/// matches what the webview would see. Returns the concatenated byte payloads
/// plus a flag indicating whether a `terminal:exit` event was observed.
fn drain_bytes_events(
    app: &tauri::App<tauri::test::MockRuntime>,
    window: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    deadline: Instant,
) -> (Vec<u8>, bool) {
    use std::sync::{Arc, Mutex};

    let collected: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let saw_exit: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));

    let collected_bytes = Arc::clone(&collected);
    let _bytes_listener = window.listen("terminal:bytes", move |ev| {
        // Event payloads arrive as a JSON string. Parse → `data: number[]`.
        let payload: Value = match serde_json::from_str(ev.payload()) {
            Ok(v) => v,
            Err(_) => return,
        };
        let Some(arr) = payload.get("data").and_then(|v| v.as_array()) else {
            return;
        };
        let mut guard = collected_bytes.lock().unwrap();
        for n in arr {
            if let Some(b) = n.as_u64() {
                guard.push(b as u8);
            }
        }
    });

    let exit_flag = Arc::clone(&saw_exit);
    let _exit_listener = app.listen("terminal:exit", move |_ev| {
        let mut guard = exit_flag.lock().unwrap();
        *guard = true;
    });

    // Spin until deadline — we can't block on `rx.recv()` here because the
    // forwarder task owns the receiver. Short sleeps keep CPU down.
    while Instant::now() < deadline {
        if *saw_exit.lock().unwrap() {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let bytes = collected.lock().unwrap().clone();
    let exit = *saw_exit.lock().unwrap();
    (bytes, exit)
}

#[test]
fn spawn_write_resize_kill_round_trip() {
    // This is the canonical DoD test: session-A spawns a shell that blocks on
    // `read`, we write `hi\n`, we resize, we kill. The byte event stream
    // must contain `GOT=hi` (proving `terminal_write` reached the child) and
    // the post-resize `stty size` output (proving `terminal_resize` took).
    let app = make_app();
    let window = make_window(&app, "session-roundtrip");
    let tid = fresh_terminal_id();

    // Use `sh -c` so we can exercise both write and resize in one shell:
    //   - block on `read` so `terminal_write` has an obvious delivery proof,
    //   - then call `stty size` after a brief sleep so `terminal_resize`
    //     has landed before the query.
    //
    // NOTE: we pass `shell: null` and let the Rust default (`$SHELL` or
    // `/bin/sh`) pick — but we need to override the program to get a
    // scripted shell. The cleanest path is to pass `sh` as the shell name
    // and rely on PATH; `TerminalSession::spawn` will find it. For the
    // argv script we exploit `ShellSpec::new` (no args) → the shell starts
    // as an interactive login, so instead we precompute and write a
    // one-liner via `terminal_write` that behaves like `sh -c`.
    //
    // Simpler: spawn the default shell, then write the one-liner as input.
    // POSIX sh reads stdin commands, so we can drive the test entirely
    // through `terminal_write`.
    invoke(
        &window,
        "terminal_spawn",
        serde_json::json!({
            "args": {
                "terminal_id": tid.clone(),
                "shell": "/bin/sh",
                "cwd": std::env::temp_dir().to_string_lossy(),
                "cols": 80,
                "rows": 24,
            }
        }),
    )
    .expect("spawn");

    // Write the scripted one-liner. The shell echoes input by default with
    // a PTY attached; `GOT=hi` appears once the shell runs the one-liner.
    // Bytes are a JSON array of u8 → the Rust side passes them unchanged
    // into `TerminalSession::write`.
    let script =
        b"stty -echo; printf 'READY\\n'; read x; stty size; printf 'GOT=%s\\n' \"$x\"; exit 0\n";
    invoke(
        &window,
        "terminal_write",
        serde_json::json!({
            "terminalId": tid.clone(),
            "data": script.to_vec(),
        }),
    )
    .expect("first write");

    // Give the shell a beat to print READY and block on `read`.
    std::thread::sleep(Duration::from_millis(400));

    // Resize — when the shell runs `stty size` below, the new rows/cols
    // must show up in the stream.
    invoke(
        &window,
        "terminal_resize",
        serde_json::json!({
            "terminalId": tid.clone(),
            "cols": 132u16,
            "rows": 50u16,
        }),
    )
    .expect("resize");

    // Deliver the line the shell is waiting for.
    invoke(
        &window,
        "terminal_write",
        serde_json::json!({
            "terminalId": tid.clone(),
            "data": b"hello\n".to_vec(),
        }),
    )
    .expect("second write");

    // Collect bytes until the shell exits (sends `terminal:exit`) or 5s
    // elapses.
    let (bytes, _saw_exit) =
        drain_bytes_events(&app, &window, Instant::now() + Duration::from_secs(5));
    let text = String::from_utf8_lossy(&bytes);

    // Proof terminal_write delivered: the shell produced GOT=hello.
    assert!(
        text.contains("GOT=hello"),
        "expected GOT=hello in byte stream; got: {text:?}"
    );

    // Proof terminal_resize landed: `stty size` prints "<rows> <cols>"
    // post-resize, so "50 132" must appear. Some environments (notably
    // when stty can't see a controlling TTY) fall back to reporting
    // something else, so accept either the resized output or the raw
    // resize ACK as evidence the path fired.
    let resize_ok = text.contains("50 132");
    assert!(
        resize_ok,
        "expected `50 132` (rows cols) after resize; got: {text:?}"
    );

    // Kill the terminal. After a successful kill the registry has no entry,
    // so a subsequent kill must fail with an "unknown terminal id" error
    // (the owner check passes since alice's label still matches — the
    // discriminator is the id lookup, not authz).
    invoke(
        &window,
        "terminal_kill",
        serde_json::json!({ "terminalId": tid.clone() }),
    )
    .expect("kill");

    let err = invoke(
        &window,
        "terminal_kill",
        serde_json::json!({ "terminalId": tid }),
    )
    .expect_err("second kill must fail — terminal already gone");
    assert!(
        err.contains("unknown terminal id"),
        "expected unknown-id error, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// Size caps (F-068-style wire bounding for terminal commands)
// ---------------------------------------------------------------------------

#[test]
fn terminal_spawn_rejects_oversize_cwd_at_command_layer() {
    let app = make_app();
    let window = make_window(&app, "session-oversize");

    // 5 KiB — above the 4 KiB cwd cap.
    let cwd = "/".repeat(5 * 1024);
    let err = invoke(
        &window,
        "terminal_spawn",
        serde_json::json!({
            "args": {
                "terminal_id": fresh_terminal_id(),
                "shell": null,
                "cwd": cwd,
                "cols": 80,
                "rows": 24,
            }
        }),
    )
    .expect_err("oversize cwd must be rejected before PTY spawn");

    assert!(
        err.contains("payload too large") && err.contains("cwd"),
        "expected cwd size-cap error, got: {err}"
    );
}

#[test]
fn terminal_write_rejects_oversize_data_at_command_layer() {
    let app = make_app();
    let window = make_window(&app, "session-data-cap");
    let tid = fresh_terminal_id();

    // Spawn a legitimate terminal first so the id is valid. Write a buffer
    // above the 64 KiB cap — the command must reject it without touching
    // the PTY.
    invoke(
        &window,
        "terminal_spawn",
        serde_json::json!({
            "args": {
                "terminal_id": tid.clone(),
                "shell": "/bin/sh",
                "cwd": std::env::temp_dir().to_string_lossy(),
                "cols": 80,
                "rows": 24,
            }
        }),
    )
    .expect("spawn");

    let oversized = vec![b'A'; 64 * 1024 + 1];
    let err = invoke(
        &window,
        "terminal_write",
        serde_json::json!({ "terminalId": tid.clone(), "data": oversized }),
    )
    .expect_err("oversize data must be rejected before PTY write");

    assert!(
        err.contains("payload too large") && err.contains("data"),
        "expected data size-cap error, got: {err}"
    );

    invoke(
        &window,
        "terminal_kill",
        serde_json::json!({ "terminalId": tid }),
    )
    .expect("kill");
}

#[test]
fn duplicate_terminal_id_is_rejected() {
    // Second spawn with the same id must error — the registry key is unique
    // within the app. We care that the rejection is loud (so the renderer
    // can surface it) rather than silent-overwrite of the previous session.
    let app = make_app();
    let window = make_window(&app, "session-dup");
    let tid = fresh_terminal_id();

    invoke(
        &window,
        "terminal_spawn",
        serde_json::json!({
            "args": {
                "terminal_id": tid.clone(),
                "shell": "/bin/sh",
                "cwd": std::env::temp_dir().to_string_lossy(),
                "cols": 80,
                "rows": 24,
            }
        }),
    )
    .expect("first spawn");

    let err = invoke(
        &window,
        "terminal_spawn",
        serde_json::json!({
            "args": {
                "terminal_id": tid.clone(),
                "shell": "/bin/sh",
                "cwd": std::env::temp_dir().to_string_lossy(),
                "cols": 80,
                "rows": 24,
            }
        }),
    )
    .expect_err("duplicate id must be rejected");

    assert!(
        err.contains("already registered"),
        "expected duplicate-id error, got: {err}"
    );

    invoke(
        &window,
        "terminal_kill",
        serde_json::json!({ "terminalId": tid }),
    )
    .expect("kill");
}
