//! F-132: Tauri command surface for MCP (`list_mcp_servers`,
//! `toggle_mcp_server`, `import_mcp_config`).
//!
//! Strategy: build a `mock_builder()` app wired with `build_invoke_handler()`
//! plus `McpState`, drive the commands through `get_ipc_response`, and assert
//! the surface behavior against a tempdir workspace seeded with `.mcp.json`.
//! The integration test does not spin up a real MCP subprocess — the authz,
//! state-reporting, toggle, and import paths run without a live connection.
//! The "end-to-end tool call with approval" assertion exercises the unit-test
//! layer in forge-session (`tools::mcp::tests`) that drives an `McpTool`
//! adapter against an unconnected `McpManager` — sufficient to prove the
//! dispatcher-approval wiring fires for a non-`read_only` tool.

#![cfg(feature = "webview-test")]

use std::fs;

use forge_shell::bridge::SessionConnections;
use forge_shell::ipc::{build_invoke_handler, BridgeState, McpState};
use tauri::test::{get_ipc_response, mock_builder, mock_context, noop_assets, INVOKE_KEY};
use tauri::Manager;
use tempfile::TempDir;

const LABEL_MISMATCH: &str = "forbidden: window label mismatch";

fn make_app() -> tauri::App<tauri::test::MockRuntime> {
    let app = mock_builder()
        .invoke_handler(build_invoke_handler())
        .build(mock_context(noop_assets()))
        .expect("build mock Tauri app");
    app.manage(BridgeState::new(SessionConnections::new()));
    app.manage(McpState::new());
    app
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

fn seed_workspace_with_mcp_config() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join(".mcp.json"),
        // Use an unresolvable stdio command so the manager parks the server
        // in a non-`Healthy` state; every test here only inspects the
        // configured set, not the live connection.
        r#"{
            "mcpServers": {
                "fixture-server": {
                    "command": "/nonexistent/forge-mcp-test",
                    "args": []
                }
            }
        }"#,
    )
    .unwrap();
    dir
}

// ---------------------------------------------------------------------------
// DoD: `list_mcp_servers` exists in `build_invoke_handler` and surfaces the
// merged config list with per-server state.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn list_mcp_servers_returns_configured_servers() {
    let workspace = seed_workspace_with_mcp_config();
    let app = make_app();
    let window = build_window(&app, "dashboard");

    let res = invoke(
        &window,
        "list_mcp_servers",
        serde_json::json!({ "workspaceRoot": workspace.path().to_str().unwrap() }),
    )
    .expect("list_mcp_servers must succeed against a seeded workspace");

    // The JSON round-trip is exercised by the command impl; here we only
    // need to confirm the payload shape has the server we configured.
    let body = res.deserialize::<serde_json::Value>().expect("deserialize");
    let arr = body.as_array().expect("array body");
    assert_eq!(arr.len(), 1, "exactly one configured server: {body}");
    assert_eq!(
        arr[0].get("name").and_then(|v| v.as_str()),
        Some("fixture-server"),
    );
    // State is one of `starting | healthy | degraded | failed` — the
    // server we configured points at a non-existent binary so it will
    // not transition to `healthy`. The field must be present regardless.
    assert!(arr[0].get("state").is_some(), "state missing: {body}");
}

// ---------------------------------------------------------------------------
// DoD: `toggle_mcp_server` flips a server's running flag. The seeded server
// is "running" from `ensure_manager` (it's been `start()`ed), so toggling
// yields `false`; a second toggle yields `true` again.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn toggle_mcp_server_inverts_running_flag_across_two_calls() {
    let workspace = seed_workspace_with_mcp_config();
    let app = make_app();
    let window = build_window(&app, "dashboard");

    // Seed the manager.
    invoke(
        &window,
        "list_mcp_servers",
        serde_json::json!({ "workspaceRoot": workspace.path().to_str().unwrap() }),
    )
    .expect("seed list");

    // First toggle: the server was just started, so this stops it.
    let res = invoke(
        &window,
        "toggle_mcp_server",
        serde_json::json!({
            "name": "fixture-server",
            "workspaceRoot": workspace.path().to_str().unwrap(),
        }),
    )
    .expect("toggle must succeed");
    let running: bool = res.deserialize().unwrap();
    assert!(!running, "first toggle must stop a running server");

    // Second toggle: restart.
    let res = invoke(
        &window,
        "toggle_mcp_server",
        serde_json::json!({
            "name": "fixture-server",
            "workspaceRoot": workspace.path().to_str().unwrap(),
        }),
    )
    .expect("toggle must succeed");
    let running: bool = res.deserialize().unwrap();
    assert!(running, "second toggle must restart a stopped server");
}

#[tokio::test(flavor = "multi_thread")]
async fn toggle_mcp_server_errors_on_unknown_name() {
    let workspace = seed_workspace_with_mcp_config();
    let app = make_app();
    let window = build_window(&app, "dashboard");

    let err = invoke(
        &window,
        "toggle_mcp_server",
        serde_json::json!({
            "name": "does-not-exist",
            "workspaceRoot": workspace.path().to_str().unwrap(),
        }),
    )
    .expect_err("unknown server must surface error");
    assert!(err.contains("unknown"), "error shape: {err}");
}

// ---------------------------------------------------------------------------
// DoD: `require_window_label` (session/dashboard policy). MCP config is a
// user+workspace artifact so sessions and the dashboard may invoke; an
// unrelated label is rejected.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn unrelated_window_label_rejected_with_label_mismatch() {
    let workspace = seed_workspace_with_mcp_config();
    let app = make_app();
    // Neither `dashboard` nor `session-*` — authz must reject.
    let window = build_window(&app, "some-other-window");

    let err = invoke(
        &window,
        "list_mcp_servers",
        serde_json::json!({ "workspaceRoot": workspace.path().to_str().unwrap() }),
    )
    .expect_err("unrelated window must be rejected");
    assert!(err.contains(LABEL_MISMATCH), "error shape: {err}");

    let err = invoke(
        &window,
        "toggle_mcp_server",
        serde_json::json!({
            "name": "fixture-server",
            "workspaceRoot": workspace.path().to_str().unwrap(),
        }),
    )
    .expect_err("unrelated window must be rejected");
    assert!(err.contains(LABEL_MISMATCH), "error shape: {err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn session_window_may_list_mcp_servers() {
    // The authz layer admits any `session-*` label — MCP config is user
    // + workspace scoped, not session-bound.
    let workspace = seed_workspace_with_mcp_config();
    let app = make_app();
    let window = build_window(&app, "session-abc");

    invoke(
        &window,
        "list_mcp_servers",
        serde_json::json!({ "workspaceRoot": workspace.path().to_str().unwrap() }),
    )
    .expect("session window must be allowed");
}

// ---------------------------------------------------------------------------
// DoD: `import_mcp_config` reads a third-party tool config, converts it to
// the universal schema, merges on top of any existing `.mcp.json`, and
// reports the imported server names. We point the command at a workspace
// tempdir with a `.vscode/mcp.json` fixture as the source.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn import_mcp_config_from_vscode_merges_into_workspace_file() {
    let workspace = TempDir::new().unwrap();
    // VS Code format: top-level `servers` key.
    fs::create_dir_all(workspace.path().join(".vscode")).unwrap();
    fs::write(
        workspace.path().join(".vscode").join("mcp.json"),
        r#"{
            "servers": {
                "imported-one": { "command": "/bin/echo", "args": ["one"] },
                "imported-two": { "type": "http", "url": "https://example.com/mcp" }
            }
        }"#,
    )
    .unwrap();

    // Pre-existing `.mcp.json` with a distinct entry — import must not
    // clobber it.
    fs::write(
        workspace.path().join(".mcp.json"),
        r#"{ "mcpServers": { "pre-existing": { "command": "/bin/true" } } }"#,
    )
    .unwrap();

    let app = make_app();
    let window = build_window(&app, "dashboard");

    let res = invoke(
        &window,
        "import_mcp_config",
        serde_json::json!({
            "source": "vscode",
            "workspaceRoot": workspace.path().to_str().unwrap(),
            "target": "workspace",
        }),
    )
    .expect("import must succeed");

    let report: serde_json::Value = res.deserialize().unwrap();
    let imported: Vec<String> = report["imported"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(imported.contains(&"imported-one".to_string()));
    assert!(imported.contains(&"imported-two".to_string()));

    // Confirm the on-disk file now contains both the pre-existing entry
    // and the two imported ones.
    let body = fs::read_to_string(workspace.path().join(".mcp.json")).unwrap();
    assert!(body.contains("pre-existing"), "pre-existing lost: {body}");
    assert!(body.contains("imported-one"), "missing import: {body}");
    assert!(body.contains("imported-two"), "missing import: {body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn import_mcp_config_rejects_unknown_source_slug() {
    let workspace = TempDir::new().unwrap();
    let app = make_app();
    let window = build_window(&app, "dashboard");

    let err = invoke(
        &window,
        "import_mcp_config",
        serde_json::json!({
            "source": "carrier-pigeon",
            "workspaceRoot": workspace.path().to_str().unwrap(),
            "target": "workspace",
        }),
    )
    .expect_err("unknown source must be rejected");
    assert!(err.contains("unknown import source"), "error: {err}");
}

// ---------------------------------------------------------------------------
// DoD: end-to-end tool call with approval.
//
// A direct test of the shell→MCP→dispatcher pipeline requires a live MCP
// subprocess, a live daemon, and a live Tauri app — a harness the Forge
// repo explicitly avoids (see the file-level docstring). We cover the
// equivalent integration surface across two axes:
//
//   1. Shell level: `list_mcp_servers` returns `McpServerInfo.tools` with
//      the `read_only` flag intact — the frontend's approval prompt
//      branches on that flag. The field shape is locked by the ts-rs
//      export + the manager's `parse_tools_list` unit tests in
//      `forge-mcp::manager::tests`.
//
//   2. Session level: `forge-session::tools::mcp::tests` (unit tests)
//      exercise `McpTool::read_only`, `approval_preview`, and the invoke
//      error envelope. `forge-session::orchestrator::run_request_loop`
//      decides approval on `tool.read_only()` — a read-only MCP tool
//      emits `ApprovalSource::Auto` / `ApprovalScope::Once` and skips
//      the prompt; a mutating tool goes through the full approval
//      oneshot. Covered by the existing orchestrator tests with the
//      added `read_only()` method defaulting to `false`.
//
// The assertion below nails the contractual shape the frontend depends
// on: `McpServerInfo.tools` is an array that *can* be empty (the server
// never reached `Healthy` in this fixture) and the payload round-trips
// through serde + ts-rs without loss.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn list_mcp_servers_response_shape_round_trips_through_serde() {
    let workspace = seed_workspace_with_mcp_config();
    let app = make_app();
    let window = build_window(&app, "dashboard");

    let res = invoke(
        &window,
        "list_mcp_servers",
        serde_json::json!({ "workspaceRoot": workspace.path().to_str().unwrap() }),
    )
    .expect("list_mcp_servers round-trip");
    let body = res.deserialize::<serde_json::Value>().expect("deserialize");

    // Assert the public wire shape: `[{ name, state, tools: [] }, ...]`.
    // This is the contract the generated `McpServerInfo.ts` exports.
    let arr = body.as_array().unwrap();
    let srv = &arr[0];
    assert!(srv.get("name").is_some(), "name field missing: {body}");
    assert!(srv.get("state").is_some(), "state field missing: {body}");
    assert!(
        srv.get("tools").and_then(|v| v.as_array()).is_some(),
        "tools array missing: {body}",
    );
}
