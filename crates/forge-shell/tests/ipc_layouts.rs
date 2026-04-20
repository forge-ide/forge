//! F-120: `read_layouts` / `write_layouts` Tauri command tests.
//!
//! Covers the two new commands end-to-end through Tauri's
//! `test::get_ipc_response` machinery:
//!
//! - Round-trip: write a layout, read it back; file content matches.
//! - Missing file: read returns the default single-pane layout without error.
//! - Corrupt file: invalid JSON on disk degrades to the default — the user
//!   should never see a blank window because a prior write scrambled the file.
//! - Authz: dashboard + any `session-*` window may invoke; other labels are
//!   rejected with the label-mismatch error (user-scoped workspace artifact,
//!   not per-session).
//!
//! Wiring mirrors `tests/approval_commands.rs` (F-036) and `tests/ipc_commands.rs`
//! (F-020 / F-052): `mock_builder()` + `WebviewWindowBuilder` with explicit
//! labels and `INVOKE_KEY` payload. `.forge/layouts.json` is materialized under
//! a `TempDir` so no test touches real workspace state.

#![cfg(feature = "webview-test")]

use forge_shell::bridge::SessionConnections;
use forge_shell::ipc::{build_invoke_handler, BridgeState};
use tauri::test::{mock_builder, mock_context, noop_assets, INVOKE_KEY};
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

fn make_foreign_window(
    app: &tauri::App<tauri::test::MockRuntime>,
) -> tauri::WebviewWindow<tauri::test::MockRuntime> {
    tauri::WebviewWindowBuilder::new(app, "rogue", tauri::WebviewUrl::App("index.html".into()))
        .build()
        .expect("mock rogue window")
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

/// Canonical sample layout — a single horizontal split between two chat leaves.
/// Covers both node variants and a per-pane state entry.
fn sample_layouts_payload(workspace_root: &str) -> serde_json::Value {
    serde_json::json!({
        "workspaceRoot": workspace_root,
        "layouts": {
            "active": "default",
            "named": {
                "default": {
                    "tree": {
                        "kind": "split",
                        "id": "split-1",
                        "direction": "h",
                        "ratio": 0.5,
                        "a": {
                            "kind": "leaf",
                            "id": "leaf-a",
                            "pane_type": "chat"
                        },
                        "b": {
                            "kind": "leaf",
                            "id": "leaf-b",
                            "pane_type": "terminal"
                        }
                    },
                    "pane_state": {
                        "leaf-a": {
                            "active_file": "src/main.rs",
                            "scroll_top": 120,
                            "terminal_pid": null
                        }
                    }
                }
            }
        }
    })
}

#[test]
fn write_then_read_round_trips_via_tauri_invoke() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_string_lossy().to_string();

    let app = make_app();
    let window = make_session_window(&app, "round-trip");

    // Write.
    let _ = invoke_ok(&window, "write_layouts", sample_layouts_payload(&root));

    // On-disk artifact lives at `<root>/.forge/layouts.json`.
    let written = dir.path().join(".forge").join("layouts.json");
    assert!(
        written.exists(),
        ".forge/layouts.json was not created under the workspace root"
    );

    // Read.
    let out = invoke_ok(
        &window,
        "read_layouts",
        serde_json::json!({ "workspaceRoot": root }),
    );
    assert_eq!(out["active"], "default");
    assert_eq!(out["named"]["default"]["tree"]["kind"], "split");
    assert_eq!(out["named"]["default"]["tree"]["a"]["pane_type"], "chat");
    assert_eq!(
        out["named"]["default"]["pane_state"]["leaf-a"]["scroll_top"],
        120
    );
}

#[test]
fn read_layouts_missing_file_returns_default_single_pane() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_string_lossy().to_string();

    let app = make_app();
    let window = make_session_window(&app, "missing");

    // No `.forge/layouts.json` on disk. The read must succeed with the default
    // single-pane shape — not surface a file-not-found error to the webview.
    let out = invoke_ok(
        &window,
        "read_layouts",
        serde_json::json!({ "workspaceRoot": root }),
    );
    assert_eq!(out["active"], "default");
    // Default is a single leaf — the tree is a leaf node, not a split.
    assert_eq!(out["named"]["default"]["tree"]["kind"], "leaf");
    assert_eq!(out["named"]["default"]["tree"]["pane_type"], "chat");
}

#[test]
fn read_layouts_corrupt_file_falls_back_to_default() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_string_lossy().to_string();

    // Materialize a corrupt `.forge/layouts.json` directly.
    let forge_dir = dir.path().join(".forge");
    std::fs::create_dir_all(&forge_dir).unwrap();
    std::fs::write(forge_dir.join("layouts.json"), b"{ this is not json")
        .expect("write corrupt layouts.json");

    let app = make_app();
    let window = make_session_window(&app, "corrupt");

    // The read must not bubble the JSON-parse error — a corrupt file would
    // otherwise leave the UI blank on every session open.
    let out = invoke_ok(
        &window,
        "read_layouts",
        serde_json::json!({ "workspaceRoot": root }),
    );
    assert_eq!(out["active"], "default");
    assert_eq!(out["named"]["default"]["tree"]["kind"], "leaf");
}

#[test]
fn read_layouts_allows_dashboard_window() {
    // Dashboard windows open the welcome view, which may need to materialize
    // the last saved layout before routing into a session. Authz gate must
    // accept the `"dashboard"` label.
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_string_lossy().to_string();

    let app = make_app();
    let window = make_dashboard_window(&app);

    let out = invoke_ok(
        &window,
        "read_layouts",
        serde_json::json!({ "workspaceRoot": root }),
    );
    assert_eq!(out["active"], "default");
}

#[test]
fn read_layouts_rejects_foreign_window_labels() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_string_lossy().to_string();

    let app = make_app();
    let window = make_foreign_window(&app);

    let err = invoke_err(
        &window,
        "read_layouts",
        serde_json::json!({ "workspaceRoot": root }),
    );

    assert!(
        err.contains(LABEL_MISMATCH),
        "expected label-mismatch error for rogue window, got: {err}"
    );
}

#[test]
fn write_layouts_rejects_foreign_window_labels() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_string_lossy().to_string();

    let app = make_app();
    let window = make_foreign_window(&app);

    let err = invoke_err(&window, "write_layouts", sample_layouts_payload(&root));

    assert!(
        err.contains(LABEL_MISMATCH),
        "expected label-mismatch error for rogue window, got: {err}"
    );

    // And — critically — the write did not land on disk.
    let written = dir.path().join(".forge").join("layouts.json");
    assert!(
        !written.exists(),
        "rejected write must not have created .forge/layouts.json"
    );
}

#[test]
fn write_layouts_creates_forge_dir_if_missing() {
    // Workspaces without an existing `.forge/` directory must still be able
    // to persist a layout on first save. The command creates the directory
    // before writing.
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_string_lossy().to_string();
    assert!(!dir.path().join(".forge").exists());

    let app = make_app();
    let window = make_session_window(&app, "fresh-ws");

    let _ = invoke_ok(&window, "write_layouts", sample_layouts_payload(&root));

    assert!(dir.path().join(".forge").join("layouts.json").exists());
}

#[test]
fn write_layouts_rejects_oversize_workspace_root() {
    // `workspace_root` reuses the same 4 KiB PATH_MAX envelope as the F-036
    // approval commands. A 5 KiB value must be rejected before the filesystem
    // is touched.
    let app = make_app();
    let window = make_session_window(&app, "oversize-root");

    let err = invoke_err(
        &window,
        "write_layouts",
        serde_json::json!({
            "workspaceRoot": "A".repeat(5 * 1024),
            "layouts": {
                "active": "default",
                "named": {
                    "default": {
                        "tree": { "kind": "leaf", "id": "root", "pane_type": "chat" },
                        "pane_state": {}
                    }
                }
            }
        }),
    );
    assert!(
        err.contains("payload too large") && err.contains("workspace_root"),
        "expected size-cap error mentioning workspace_root, got: {err}"
    );
}
