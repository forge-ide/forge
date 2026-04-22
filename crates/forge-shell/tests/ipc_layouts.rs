//! F-120: `read_layouts` / `write_layouts` Tauri command tests.
//!
//! Covers the two commands end-to-end through Tauri's
//! `test::get_ipc_response` machinery:
//!
//! - Round-trip: write a layout, read it back; file content matches.
//! - Missing file: read returns the default single-pane layout without error.
//! - Corrupt file: invalid JSON on disk degrades to the default — the user
//!   should never see a blank window because a prior write scrambled the file.
//! - Authz: dashboard + any `session-*` window may invoke; other labels are
//!   rejected with the label-mismatch error.
//!
//! F-349: workspace-root validation regression tests are appended at the
//! bottom. Session-* callers have the webview-supplied value replaced by the
//! server-side cache; dashboard callers must supply a registry-listed path.
//!
//! Wiring mirrors `tests/approval_commands.rs` (F-036) and
//! `tests/ipc_commands.rs` (F-020 / F-052): `mock_builder()` +
//! `WebviewWindowBuilder` with explicit labels and `INVOKE_KEY` payload.

#![cfg(feature = "webview-test")]

use forge_core::workspaces::{write_workspaces, WorkspaceEntry};
use forge_shell::bridge::SessionConnections;
use forge_shell::ipc::{build_invoke_handler, BridgeState};
use tauri::test::{mock_builder, mock_context, noop_assets, INVOKE_KEY};
use tauri::Manager;
use tempfile::TempDir;

const LABEL_MISMATCH: &str = "forbidden: window label mismatch";

/// Build a mock app with the workspace-root cache primed for `session_id`.
async fn make_app_with_workspace(
    workspace: &std::path::Path,
    session_id: &str,
) -> tauri::App<tauri::test::MockRuntime> {
    let connections = SessionConnections::new();
    connections
        .prime_workspace_root_for_test(
            session_id.to_string(),
            std::fs::canonicalize(workspace).expect("canonicalize workspace"),
        )
        .await;
    let app = mock_builder()
        .invoke_handler(build_invoke_handler())
        .build(mock_context(noop_assets()))
        .expect("build mock Tauri app");
    app.manage(BridgeState::new(connections));
    app
}

/// Build a mock app whose workspace-root cache is intentionally empty. Used
/// to verify the "not connected" error path and for authz tests that never
/// reach the workspace-root resolution.
fn make_app_empty_cache() -> tauri::App<tauri::test::MockRuntime> {
    let app = mock_builder()
        .invoke_handler(build_invoke_handler())
        .build(mock_context(noop_assets()))
        .expect("build mock Tauri app");
    app.manage(BridgeState::new(SessionConnections::new()));
    app
}

/// Build a mock app wired for dashboard-caller tests. The workspaces registry
/// is seeded with `workspace_paths` so the F-349 registry-validation path
/// can accept those paths and reject others.
async fn make_app_with_registry(
    workspace_paths: &[&std::path::Path],
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

    let app = mock_builder()
        .invoke_handler(build_invoke_handler())
        .build(mock_context(noop_assets()))
        .expect("build mock Tauri app");
    app.manage(BridgeState::with_test_workspaces_toml(
        SessionConnections::new(),
        toml_path,
    ));
    (app, registry_dir)
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

// ---------------------------------------------------------------------------
// Functional correctness
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn write_then_read_round_trips_via_tauri_invoke() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_string_lossy().to_string();

    let app = make_app_with_workspace(dir.path(), "round-trip").await;
    let window = make_session_window(&app, "round-trip");

    let _ = invoke_ok(&window, "write_layouts", sample_layouts_payload(&root));

    let written = dir.path().join(".forge").join("layouts.json");
    assert!(
        written.exists(),
        ".forge/layouts.json was not created under the workspace root"
    );

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

#[tokio::test(flavor = "multi_thread")]
async fn read_layouts_missing_file_returns_default_single_pane() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_string_lossy().to_string();

    let app = make_app_with_workspace(dir.path(), "missing").await;
    let window = make_session_window(&app, "missing");

    let out = invoke_ok(
        &window,
        "read_layouts",
        serde_json::json!({ "workspaceRoot": root }),
    );
    assert_eq!(out["active"], "default");
    assert_eq!(out["named"]["default"]["tree"]["kind"], "leaf");
    assert_eq!(out["named"]["default"]["tree"]["pane_type"], "chat");
}

#[tokio::test(flavor = "multi_thread")]
async fn read_layouts_corrupt_file_falls_back_to_default() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_string_lossy().to_string();

    let forge_dir = dir.path().join(".forge");
    std::fs::create_dir_all(&forge_dir).unwrap();
    std::fs::write(forge_dir.join("layouts.json"), b"{ this is not json")
        .expect("write corrupt layouts.json");

    let app = make_app_with_workspace(dir.path(), "corrupt").await;
    let window = make_session_window(&app, "corrupt");

    let out = invoke_ok(
        &window,
        "read_layouts",
        serde_json::json!({ "workspaceRoot": root }),
    );
    assert_eq!(out["active"], "default");
    assert_eq!(out["named"]["default"]["tree"]["kind"], "leaf");
}

#[tokio::test(flavor = "multi_thread")]
async fn read_layouts_allows_dashboard_window() {
    // Dashboard windows must be accepted when the supplied path is registered.
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_string_lossy().to_string();

    let (app, _registry) = make_app_with_registry(&[dir.path()]).await;
    let window = make_dashboard_window(&app);

    let out = invoke_ok(
        &window,
        "read_layouts",
        serde_json::json!({ "workspaceRoot": root }),
    );
    assert_eq!(out["active"], "default");
}

#[tokio::test(flavor = "multi_thread")]
async fn read_layouts_rejects_foreign_window_labels() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_string_lossy().to_string();

    let app = make_app_empty_cache();
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

#[tokio::test(flavor = "multi_thread")]
async fn write_layouts_rejects_foreign_window_labels() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_string_lossy().to_string();

    let app = make_app_empty_cache();
    let window = make_foreign_window(&app);

    let err = invoke_err(&window, "write_layouts", sample_layouts_payload(&root));
    assert!(
        err.contains(LABEL_MISMATCH),
        "expected label-mismatch error for rogue window, got: {err}"
    );

    let written = dir.path().join(".forge").join("layouts.json");
    assert!(
        !written.exists(),
        "rejected write must not have created .forge/layouts.json"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn write_layouts_creates_forge_dir_if_missing() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_string_lossy().to_string();
    assert!(!dir.path().join(".forge").exists());

    let app = make_app_with_workspace(dir.path(), "fresh-ws").await;
    let window = make_session_window(&app, "fresh-ws");

    let _ = invoke_ok(&window, "write_layouts", sample_layouts_payload(&root));

    assert!(dir.path().join(".forge").join("layouts.json").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn write_layouts_is_atomic_via_tmp_and_rename() {
    // F-363: write to `<path>.tmp`, then rename into place.
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_string_lossy().to_string();

    let forge_dir = dir.path().join(".forge");
    std::fs::create_dir_all(&forge_dir).unwrap();
    let tmp_path = forge_dir.join("layouts.json.tmp");
    std::fs::write(&tmp_path, b"{ partial-crash-residue").expect("seed stale layouts.json.tmp");

    let app = make_app_with_workspace(dir.path(), "atomic").await;
    let window = make_session_window(&app, "atomic");

    let _ = invoke_ok(&window, "write_layouts", sample_layouts_payload(&root));

    let final_path = forge_dir.join("layouts.json");
    assert!(final_path.exists(), "final layouts.json must exist");
    assert!(
        !tmp_path.exists(),
        "layouts.json.tmp must be consumed by rename, not left behind"
    );

    let bytes = std::fs::read(&final_path).expect("read final layouts.json");
    let parsed: serde_json::Value =
        serde_json::from_slice(&bytes).expect("final layouts.json is valid JSON");
    assert_eq!(parsed["active"], "default");
    assert_eq!(parsed["named"]["default"]["tree"]["kind"], "split");
}

#[tokio::test(flavor = "multi_thread")]
async fn write_layouts_crash_mid_write_leaves_last_valid_on_disk() {
    // F-363 DoD: a partial tmp left behind must not affect the canonical file.
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_string_lossy().to_string();

    let app = make_app_with_workspace(dir.path(), "crash-mid-write").await;
    let window = make_session_window(&app, "crash-mid-write");

    let _ = invoke_ok(&window, "write_layouts", sample_layouts_payload(&root));

    let forge_dir = dir.path().join(".forge");
    let tmp_path = forge_dir.join("layouts.json.tmp");
    std::fs::write(&tmp_path, b"{ mid-write-crash").expect("seed crash tmp");

    let out = invoke_ok(
        &window,
        "read_layouts",
        serde_json::json!({ "workspaceRoot": root }),
    );
    assert_eq!(out["active"], "default");
    assert_eq!(out["named"]["default"]["tree"]["kind"], "split");
    assert_eq!(out["named"]["default"]["tree"]["a"]["pane_type"], "chat");
}

#[tokio::test(flavor = "multi_thread")]
async fn write_layouts_rejects_oversize_workspace_root() {
    let app = make_app_empty_cache();
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

// ---------------------------------------------------------------------------
// F-349: workspace-root authorization regression tests
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn session_window_ignores_forged_workspace_root_and_uses_cached() {
    // A session-* webview supplying a forged `workspace_root` must have its
    // value replaced by the server-side cached workspace root.
    let real_ws = TempDir::new().unwrap();
    let forged_ws = TempDir::new().unwrap();

    // Write a marker layout only in the real workspace.
    let real_forge_dir = real_ws.path().join(".forge");
    std::fs::create_dir_all(&real_forge_dir).unwrap();
    let marker = serde_json::json!({
        "active": "real",
        "named": {
            "real": {
                "tree": { "kind": "leaf", "id": "root", "pane_type": "chat" },
                "pane_state": {}
            }
        }
    });
    std::fs::write(
        real_forge_dir.join("layouts.json"),
        serde_json::to_vec_pretty(&marker).unwrap(),
    )
    .unwrap();

    let app = make_app_with_workspace(real_ws.path(), "sess-a").await;
    let window = make_session_window(&app, "sess-a");

    // Invoke with the forged path.
    let forged_root = forged_ws.path().to_string_lossy().to_string();
    let out = invoke_ok(
        &window,
        "read_layouts",
        serde_json::json!({ "workspaceRoot": forged_root }),
    );
    assert_eq!(
        out["active"], "real",
        "session-* caller must read from the cached workspace, not the forged path"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn session_window_forged_write_lands_in_cached_workspace() {
    // A session-* webview supplying a forged `workspace_root` to `write_layouts`
    // must have the write land in the real (cached) workspace.
    let real_ws = TempDir::new().unwrap();
    let forged_ws = TempDir::new().unwrap();

    let app = make_app_with_workspace(real_ws.path(), "sess-b").await;
    let window = make_session_window(&app, "sess-b");

    let forged_root = forged_ws.path().to_string_lossy().to_string();
    let _ = invoke_ok(&window, "write_layouts", sample_layouts_payload(&forged_root));

    assert!(
        real_ws.path().join(".forge").join("layouts.json").exists(),
        "write must land in the real (cached) workspace"
    );
    assert!(
        !forged_ws.path().join(".forge").exists(),
        "forged workspace must not have been touched"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn session_window_with_empty_cache_returns_not_connected_error() {
    // When a session-* window invokes before `session_hello` has primed the
    // cache, the command must return a "not connected" error rather than reading
    // from the webview-supplied path.
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_string_lossy().to_string();

    let app = make_app_empty_cache();
    let window = make_session_window(&app, "unprime");

    let err = invoke_err(
        &window,
        "read_layouts",
        serde_json::json!({ "workspaceRoot": root }),
    );
    assert!(
        err.contains("not connected"),
        "expected 'not connected' error for unprimed session, got: {err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dashboard_window_rejects_workspace_root_not_in_registry() {
    // A dashboard caller supplying a path not in the workspaces registry must
    // be rejected — cannot read/write arbitrary on-disk paths.
    let registered_ws = TempDir::new().unwrap();
    let unregistered_ws = TempDir::new().unwrap();

    let (app, _registry) = make_app_with_registry(&[registered_ws.path()]).await;
    let window = make_dashboard_window(&app);

    let unregistered_root = unregistered_ws.path().to_string_lossy().to_string();
    let err = invoke_err(
        &window,
        "read_layouts",
        serde_json::json!({ "workspaceRoot": unregistered_root }),
    );
    assert!(
        err.contains("not in registry"),
        "expected registry-rejection error, got: {err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dashboard_window_accepts_registered_workspace_root() {
    // A dashboard caller supplying a path in the registry must succeed.
    let workspace = TempDir::new().unwrap();

    let (app, _registry) = make_app_with_registry(&[workspace.path()]).await;
    let window = make_dashboard_window(&app);

    let root = workspace.path().to_string_lossy().to_string();
    let out = invoke_ok(
        &window,
        "read_layouts",
        serde_json::json!({ "workspaceRoot": root }),
    );
    assert_eq!(out["active"], "default");
}

#[tokio::test(flavor = "multi_thread")]
async fn dashboard_window_rejects_nonexistent_path() {
    // A path that doesn't exist cannot be canonicalized and must be rejected.
    let (app, _registry) = make_app_with_registry(&[]).await;
    let window = make_dashboard_window(&app);

    let err = invoke_err(
        &window,
        "read_layouts",
        serde_json::json!({ "workspaceRoot": "/tmp/forge-test-nonexistent-99999" }),
    );
    assert!(
        err.contains("not found on disk") || err.contains("not in registry"),
        "expected path-not-found or registry error, got: {err}"
    );
}
