//! F-122 filesystem Tauri command tests.
//!
//! Covers the three new commands — `read_file`, `write_file`, `tree` —
//! through Tauri's `test::get_ipc_response` machinery. Three invariants
//! are exercised end-to-end:
//!
//! 1. **Window-label authz.** Dashboard / cross-session invokes are
//!    rejected with the F-051 label-mismatch error before the bridge sees
//!    the frame.
//! 2. **Path sandboxing via `forge-fs`.** A `path` or `root` outside the
//!    session's workspace is rejected with a path-denied error — the
//!    `forge-fs` allowlist is the enforcement authority.
//! 3. **Server-side workspace-root cache (the F-122 rev-2 fix).** The
//!    command signature does NOT accept `workspace_root` from the webview;
//!    it is looked up server-side from `SessionConnections`, populated at
//!    `session_hello` time. Tests prime the cache through the
//!    `webview-test`-gated `prime_workspace_root_for_test` seam (no live
//!    UDS) and verify:
//!       - a path that would be allowed under a *different* workspace still
//!         gets rejected (the cached value is authoritative);
//!       - a session whose cache is empty (no `hello` yet) returns a clear
//!         "not connected" error instead of a cryptic path-denied.

#![cfg(feature = "webview-test")]

use std::fs;

use forge_shell::bridge::SessionConnections;
use forge_shell::ipc::{build_invoke_handler, BridgeState};
use tauri::test::{get_ipc_response, mock_builder, mock_context, noop_assets, INVOKE_KEY};
use tauri::Manager;
use tempfile::TempDir;

const LABEL_MISMATCH: &str = "forbidden: window label mismatch";
const TEST_SESSION: &str = "abcdef0123456789";

/// Build a Tauri mock app with the F-122 commands and a session connections
/// registry that a test has primed. Async because the cache-prime crosses a
/// `tokio::sync::Mutex`; the `#[tokio::test(flavor = "multi_thread")]`
/// attribute already provides the driving runtime, so nested
/// `block_on` would fail.
async fn make_app_with_workspace(
    workspace: &std::path::Path,
    session_id: &str,
) -> (tauri::App<tauri::test::MockRuntime>, SessionConnections) {
    let app = mock_builder()
        .invoke_handler(build_invoke_handler())
        .build(mock_context(noop_assets()))
        .expect("build mock Tauri app");
    let connections = SessionConnections::new();
    connections
        .prime_workspace_root_for_test(
            session_id.to_string(),
            fs::canonicalize(workspace).expect("canonicalize workspace"),
        )
        .await;
    app.manage(BridgeState::new(connections.clone()));
    (app, connections)
}

fn make_app_empty_cache() -> tauri::App<tauri::test::MockRuntime> {
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

// ---------------------------------------------------------------------------
// read_file
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn read_file_returns_content_for_a_workspace_file() {
    let workspace = TempDir::new().unwrap();
    let canonical_ws = fs::canonicalize(workspace.path()).unwrap();
    let file = canonical_ws.join("hello.txt");
    fs::write(&file, "hello\n").unwrap();

    let (app, _conn) = make_app_with_workspace(workspace.path(), TEST_SESSION).await;
    let window = make_session_window(&app, TEST_SESSION);

    let result = invoke_ok(
        &window,
        "read_file",
        serde_json::json!({
            "sessionId": TEST_SESSION,
            "path": file,
        }),
    );
    assert_eq!(result["content"], serde_json::json!("hello\n"));
    assert_eq!(result["bytes"], serde_json::json!(6));
}

#[tokio::test(flavor = "multi_thread")]
async fn read_file_rejects_paths_outside_the_workspace() {
    let workspace = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    let canonical_outside = fs::canonicalize(outside.path()).unwrap();
    let victim = canonical_outside.join("secret.txt");
    fs::write(&victim, "do not leak").unwrap();

    let (app, _conn) = make_app_with_workspace(workspace.path(), TEST_SESSION).await;
    let window = make_session_window(&app, TEST_SESSION);

    let err = invoke_err(
        &window,
        "read_file",
        serde_json::json!({
            "sessionId": TEST_SESSION,
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

    let (app, _conn) = make_app_with_workspace(workspace.path(), TEST_SESSION).await;
    let window = make_dashboard_window(&app);

    let err = invoke_err(
        &window,
        "read_file",
        serde_json::json!({
            "sessionId": TEST_SESSION,
            "path": file,
        }),
    );
    assert!(
        err.contains(LABEL_MISMATCH),
        "dashboard window must be rejected, got: {err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn read_file_returns_not_connected_when_cache_is_empty() {
    // No prime: the session has never called `session_hello`. The command
    // must return a "not connected" error, NOT a path-denied or panic.
    // The specific path we ask for doesn't matter — the cache lookup
    // fails before `forge-fs` ever runs.
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("hello.txt");
    fs::write(&file, "hi").unwrap();

    let app = make_app_empty_cache();
    let window = make_session_window(&app, TEST_SESSION);

    let err = invoke_err(
        &window,
        "read_file",
        serde_json::json!({
            "sessionId": TEST_SESSION,
            "path": file,
        }),
    );
    assert!(
        err.contains("not connected") && err.contains("session_hello"),
        "expected 'not connected' error, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// write_file
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn write_file_creates_a_new_file_and_roundtrips() {
    let workspace = TempDir::new().unwrap();
    let canonical_ws = fs::canonicalize(workspace.path()).unwrap();
    let file = canonical_ws.join("new.txt");
    let payload = "created by write_file\n";
    let bytes_payload: Vec<u8> = payload.as_bytes().to_vec();

    let (app, _conn) = make_app_with_workspace(workspace.path(), TEST_SESSION).await;
    let window = make_session_window(&app, TEST_SESSION);

    invoke_ok(
        &window,
        "write_file",
        serde_json::json!({
            "sessionId": TEST_SESSION,
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
    let outside = TempDir::new().unwrap();
    let canonical_outside = fs::canonicalize(outside.path()).unwrap();
    let victim = canonical_outside.join("attacker-written.txt");

    let (app, _conn) = make_app_with_workspace(workspace.path(), TEST_SESSION).await;
    let window = make_session_window(&app, TEST_SESSION);

    let err = invoke_err(
        &window,
        "write_file",
        serde_json::json!({
            "sessionId": TEST_SESSION,
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

    let (app, _conn) = make_app_with_workspace(workspace.path(), TEST_SESSION).await;
    let window = make_dashboard_window(&app);

    let err = invoke_err(
        &window,
        "write_file",
        serde_json::json!({
            "sessionId": TEST_SESSION,
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

// ---------------------------------------------------------------------------
// Server-side workspace-root authority (F-122 mandatory fix 1)
// ---------------------------------------------------------------------------

/// This is the regression test the maintainer asked for in the PR #279
/// review. The cache is primed with `workspace_A`; an attempt to read a
/// file inside a *different* workspace_B (a path the webview cannot name
/// through a parameter anymore — the param was dropped) must still be
/// rejected by the `forge-fs` allowlist because the allowlist is derived
/// from the cache, not a claim.
///
/// Because the webview cannot supply `workspace_root` at all, the attack
/// we guard against is reframed as: "no matter what `path` you pick, the
/// cache is the authority." A path inside `workspace_B` is path-denied;
/// a path inside `workspace_A` is allowed. The test asserts both.
#[tokio::test(flavor = "multi_thread")]
async fn cached_workspace_root_is_authoritative_even_when_webview_probes_other_dirs() {
    let workspace_a = TempDir::new().unwrap();
    let canonical_a = fs::canonicalize(workspace_a.path()).unwrap();
    let file_a = canonical_a.join("trusted.txt");
    fs::write(&file_a, "ok\n").unwrap();

    let workspace_b = TempDir::new().unwrap();
    let canonical_b = fs::canonicalize(workspace_b.path()).unwrap();
    let file_b = canonical_b.join("secret.txt");
    fs::write(&file_b, "should not escape").unwrap();

    // Cache is primed with workspace_A only.
    let (app, _conn) = make_app_with_workspace(workspace_a.path(), TEST_SESSION).await;
    let window = make_session_window(&app, TEST_SESSION);

    // Attack path: read workspace_B's file. No matter what the webview
    // does, workspace_root is looked up server-side and comes back as
    // workspace_A → file_b is outside the allowlist → path-denied.
    let err = invoke_err(
        &window,
        "read_file",
        serde_json::json!({
            "sessionId": TEST_SESSION,
            "path": file_b,
        }),
    );
    assert!(
        err.contains("not allowed") || err.contains("PathDenied"),
        "cached workspace A must reject a read into workspace B, got: {err}"
    );

    // Sanity check: a path inside workspace_A *does* succeed through the
    // same invoke path, proving the rejection above was about the path,
    // not about the invoke being broken.
    let ok = invoke_ok(
        &window,
        "read_file",
        serde_json::json!({
            "sessionId": TEST_SESSION,
            "path": file_a,
        }),
    );
    assert_eq!(ok["content"], serde_json::json!("ok\n"));
}

// ---------------------------------------------------------------------------
// tree
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn tree_lists_files_inside_the_workspace() {
    let workspace = TempDir::new().unwrap();
    let canonical_ws = fs::canonicalize(workspace.path()).unwrap();
    fs::write(canonical_ws.join("a.txt"), "a").unwrap();
    fs::create_dir(canonical_ws.join("sub")).unwrap();
    fs::write(canonical_ws.join("sub/b.txt"), "b").unwrap();

    let (app, _conn) = make_app_with_workspace(workspace.path(), TEST_SESSION).await;
    let window = make_session_window(&app, TEST_SESSION);

    let result = invoke_ok(
        &window,
        "tree",
        serde_json::json!({
            "sessionId": TEST_SESSION,
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
    let outside = TempDir::new().unwrap();
    let canonical_outside = fs::canonicalize(outside.path()).unwrap();

    let (app, _conn) = make_app_with_workspace(workspace.path(), TEST_SESSION).await;
    let window = make_session_window(&app, TEST_SESSION);

    let err = invoke_err(
        &window,
        "tree",
        serde_json::json!({
            "sessionId": TEST_SESSION,
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

    // Cache a session "A"; window is for session "A"; but the invoke
    // claims to be for session "B". The label check fails before any
    // cache lookup happens.
    let (app, _conn) = make_app_with_workspace(workspace.path(), "A").await;
    let window = make_session_window(&app, "A");

    let err = invoke_err(
        &window,
        "tree",
        serde_json::json!({
            "sessionId": "B",
            "root": canonical_ws,
            "depth": 4,
        }),
    );
    assert!(
        err.contains(LABEL_MISMATCH),
        "cross-session invoke must be rejected, got: {err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tree_excludes_gitignored_entries() {
    let workspace = TempDir::new().unwrap();
    let canonical_ws = fs::canonicalize(workspace.path()).unwrap();
    fs::write(canonical_ws.join(".gitignore"), "node_modules/\n*.log\n").unwrap();
    fs::write(canonical_ws.join("app.ts"), "").unwrap();
    fs::write(canonical_ws.join("err.log"), "leak").unwrap();
    fs::create_dir(canonical_ws.join("node_modules")).unwrap();
    fs::write(canonical_ws.join("node_modules/pkg.js"), "").unwrap();

    let (app, _conn) = make_app_with_workspace(workspace.path(), TEST_SESSION).await;
    let window = make_session_window(&app, TEST_SESSION);

    let result = invoke_ok(
        &window,
        "tree",
        serde_json::json!({
            "sessionId": TEST_SESSION,
            "root": canonical_ws,
            "depth": 4,
        }),
    );
    let children = result["children"].as_array().expect("children array");
    let names: Vec<&str> = children
        .iter()
        .map(|n| n["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"app.ts"), "app.ts must remain: {names:?}");
    assert!(
        !names.contains(&"err.log"),
        "*.log must be excluded from sidebar tree: {names:?}"
    );
    assert!(
        !names.contains(&"node_modules"),
        "node_modules/ must be excluded: {names:?}"
    );
}

// ---------------------------------------------------------------------------
// rename_path (F-126)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn rename_path_moves_file_within_workspace() {
    let workspace = TempDir::new().unwrap();
    let canonical_ws = fs::canonicalize(workspace.path()).unwrap();
    let from = canonical_ws.join("old.txt");
    let to = canonical_ws.join("new.txt");
    fs::write(&from, "contents").unwrap();

    let (app, _conn) = make_app_with_workspace(workspace.path(), TEST_SESSION).await;
    let window = make_session_window(&app, TEST_SESSION);

    invoke_ok(
        &window,
        "rename_path",
        serde_json::json!({
            "sessionId": TEST_SESSION,
            "from": from,
            "to": to,
        }),
    );
    assert!(!from.exists());
    assert_eq!(fs::read_to_string(&to).unwrap(), "contents");
}

#[tokio::test(flavor = "multi_thread")]
async fn rename_path_rejects_destination_outside_workspace() {
    let workspace = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    let canonical_ws = fs::canonicalize(workspace.path()).unwrap();
    let canonical_outside = fs::canonicalize(outside.path()).unwrap();
    let from = canonical_ws.join("inside.txt");
    let to = canonical_outside.join("escape.txt");
    fs::write(&from, "x").unwrap();

    let (app, _conn) = make_app_with_workspace(workspace.path(), TEST_SESSION).await;
    let window = make_session_window(&app, TEST_SESSION);

    let err = invoke_err(
        &window,
        "rename_path",
        serde_json::json!({
            "sessionId": TEST_SESSION,
            "from": from,
            "to": to,
        }),
    );
    assert!(
        err.contains("not allowed") || err.contains("PathDenied"),
        "must reject out-of-workspace destination: {err}"
    );
    assert!(from.exists(), "source must remain when rename denied");
    assert!(!to.exists(), "destination must not be created on denial");
}

#[tokio::test(flavor = "multi_thread")]
async fn rename_path_rejects_dashboard_window() {
    let workspace = TempDir::new().unwrap();
    let canonical_ws = fs::canonicalize(workspace.path()).unwrap();
    let from = canonical_ws.join("a.txt");
    let to = canonical_ws.join("b.txt");
    fs::write(&from, "x").unwrap();

    let (app, _conn) = make_app_with_workspace(workspace.path(), TEST_SESSION).await;
    let window = make_dashboard_window(&app);

    let err = invoke_err(
        &window,
        "rename_path",
        serde_json::json!({
            "sessionId": TEST_SESSION,
            "from": from,
            "to": to,
        }),
    );
    assert!(
        err.contains(LABEL_MISMATCH),
        "dashboard must not invoke rename_path: {err}"
    );
    assert!(from.exists(), "source must remain");
}

// ---------------------------------------------------------------------------
// delete_path (F-126)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn delete_path_removes_file_inside_workspace() {
    let workspace = TempDir::new().unwrap();
    let canonical_ws = fs::canonicalize(workspace.path()).unwrap();
    let target = canonical_ws.join("doomed.txt");
    fs::write(&target, "x").unwrap();

    let (app, _conn) = make_app_with_workspace(workspace.path(), TEST_SESSION).await;
    let window = make_session_window(&app, TEST_SESSION);

    invoke_ok(
        &window,
        "delete_path",
        serde_json::json!({
            "sessionId": TEST_SESSION,
            "path": target,
        }),
    );
    assert!(!target.exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_path_rejects_paths_outside_workspace() {
    let workspace = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    let canonical_outside = fs::canonicalize(outside.path()).unwrap();
    let victim = canonical_outside.join("keep.txt");
    fs::write(&victim, "important").unwrap();

    let (app, _conn) = make_app_with_workspace(workspace.path(), TEST_SESSION).await;
    let window = make_session_window(&app, TEST_SESSION);

    let err = invoke_err(
        &window,
        "delete_path",
        serde_json::json!({
            "sessionId": TEST_SESSION,
            "path": victim,
        }),
    );
    assert!(
        err.contains("not allowed") || err.contains("PathDenied"),
        "must reject out-of-workspace path: {err}"
    );
    assert!(victim.exists(), "victim must remain when delete denied");
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_path_rejects_dashboard_window() {
    let workspace = TempDir::new().unwrap();
    let canonical_ws = fs::canonicalize(workspace.path()).unwrap();
    let target = canonical_ws.join("forbidden.txt");
    fs::write(&target, "x").unwrap();

    let (app, _conn) = make_app_with_workspace(workspace.path(), TEST_SESSION).await;
    let window = make_dashboard_window(&app);

    let err = invoke_err(
        &window,
        "delete_path",
        serde_json::json!({
            "sessionId": TEST_SESSION,
            "path": target,
        }),
    );
    assert!(
        err.contains(LABEL_MISMATCH),
        "dashboard must not invoke delete_path: {err}"
    );
    assert!(target.exists(), "target must remain");
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_path_returns_not_connected_when_cache_is_empty() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("hi.txt");
    fs::write(&file, "hi").unwrap();

    let app = make_app_empty_cache();
    let window = make_session_window(&app, TEST_SESSION);

    let err = invoke_err(
        &window,
        "delete_path",
        serde_json::json!({
            "sessionId": TEST_SESSION,
            "path": file,
        }),
    );
    assert!(
        err.contains("not connected") && err.contains("session_hello"),
        "expected 'not connected' error, got: {err}"
    );
    assert!(file.exists(), "file must remain on not-connected");
}

// ---------------------------------------------------------------------------
// F-365: sandbox-escape coverage on the shell side.
//
// The existing tests above cover out-of-workspace *destinations* for rename
// and out-of-workspace *paths* for delete. The F-365 review flagged that the
// `from` side of `rename_path` was not exercised, and that neither command
// had an assertion aligned with the concrete attack shape called out in the
// issue:
//
//     rename_path(session_id, "/etc/passwd", "/tmp/x")
//
// i.e. both endpoints are absolute paths outside the primed workspace_root.
// These tests pin that exact shape so a regression that loosens the
// `forge-fs` allowlist check cannot land silently.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn rename_path_rejects_absolute_paths_outside_primed_workspace() {
    // Cache is primed with an empty workspace; the webview supplies absolute
    // paths pointing completely outside it. Both endpoints must be path-denied
    // via the `forge-fs` allowlist, and neither path may be created, moved,
    // or otherwise touched on disk.
    let workspace = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    let canonical_outside = fs::canonicalize(outside.path()).unwrap();
    let from = canonical_outside.join("source.txt");
    let to = canonical_outside.join("destination.txt");
    fs::write(&from, "source").unwrap();

    let (app, _conn) = make_app_with_workspace(workspace.path(), TEST_SESSION).await;
    let window = make_session_window(&app, TEST_SESSION);

    let err = invoke_err(
        &window,
        "rename_path",
        serde_json::json!({
            "sessionId": TEST_SESSION,
            "from": from,
            "to": to,
        }),
    );
    assert!(
        err.contains("not allowed") || err.contains("PathDenied"),
        "rename_path with an out-of-workspace `from` must be denied by forge-fs, got: {err}"
    );
    assert!(
        from.exists(),
        "source outside workspace must remain untouched"
    );
    assert!(
        !to.exists(),
        "destination outside workspace must not be created"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_path_rejects_absolute_path_outside_primed_workspace() {
    // Mirrors the `rename_path` absolute-path escape: a delete targeting an
    // absolute path fully outside the session's workspace must be rejected,
    // and the target must remain on disk.
    let workspace = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    let canonical_outside = fs::canonicalize(outside.path()).unwrap();
    let victim = canonical_outside.join("keep.txt");
    fs::write(&victim, "do not delete").unwrap();

    let (app, _conn) = make_app_with_workspace(workspace.path(), TEST_SESSION).await;
    let window = make_session_window(&app, TEST_SESSION);

    let err = invoke_err(
        &window,
        "delete_path",
        serde_json::json!({
            "sessionId": TEST_SESSION,
            "path": victim,
        }),
    );
    assert!(
        err.contains("not allowed") || err.contains("PathDenied"),
        "delete_path with an out-of-workspace absolute path must be denied by forge-fs, got: {err}"
    );
    assert!(victim.exists(), "victim outside workspace must remain");
    assert_eq!(
        fs::read_to_string(&victim).unwrap(),
        "do not delete",
        "victim contents must be unchanged"
    );
}
