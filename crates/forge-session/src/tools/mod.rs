//! Tool dispatch: name → handler routing for orchestrator tool calls.

use crate::sandbox::ChildRegistry;
use forge_core::ApprovalPreview;

pub mod fs_edit;
pub mod fs_read;
pub mod fs_write;
pub mod shell_exec;

pub use fs_edit::FsEditTool;
pub use fs_read::FsReadTool;
pub use fs_write::FsWriteTool;
pub use shell_exec::ShellExecTool;

#[derive(Default)]
pub struct ToolCtx {
    pub allowed_paths: Vec<String>,
    /// Workspace root for tools that spawn subprocesses (e.g. `shell.exec`).
    pub workspace_root: Option<std::path::PathBuf>,
    /// Registry of live sandboxed children — populated for tools that spawn
    /// processes so session shutdown can kill them.
    pub child_registry: Option<ChildRegistry>,
}

pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn approval_preview(&self, args: &serde_json::Value) -> ApprovalPreview;
    fn invoke(&self, args: &serde_json::Value, ctx: &ToolCtx) -> serde_json::Value;
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ToolError {
    #[error("tool '{0}' is already registered")]
    DuplicateName(String),
    #[error("unknown tool '{0}'")]
    UnknownTool(String),
    /// A required string argument was absent or the value was not a JSON
    /// string. Empty strings are accepted — see [`get_required_str`] for the
    /// rationale. `Display` shape is
    /// `tool.{tool}: missing required parameter '{arg}'` and is asserted by
    /// IPC-level regression tests; treat it as contractual. F-075 will
    /// broaden the extractor to cover optional / non-string arguments;
    /// keep this variant minimal so that extension stays additive.
    #[error("tool.{tool}: missing required parameter '{arg}'")]
    MissingRequiredArg { tool: String, arg: String },
}

/// Extract a required string argument from a tool-call's JSON args object.
///
/// Returns `Err(ToolError::MissingRequiredArg { tool, arg: key })` when the
/// key is absent or the value is not a JSON string. Empty strings are
/// **accepted** — `fs.write` with `{"content": ""}` is a legitimate
/// "truncate file to zero bytes" operation, and rejecting it here would
/// regress that behaviour with a misleading "missing parameter" error
/// (the parameter is supplied; it is just empty). Tools that need to
/// additionally reject empty (e.g. `shell.exec` on `command`) layer that
/// check on top of this helper.
///
/// F-075 will refactor this into a broader extractor (optional values,
/// non-string types). Keep the signature narrow so that extension is purely
/// additive — do not bake in a generic here.
pub fn get_required_str<'a>(
    args: &'a serde_json::Value,
    tool: &str,
    key: &str,
) -> Result<&'a str, ToolError> {
    match args.get(key).and_then(|v| v.as_str()) {
        Some(s) => Ok(s),
        None => Err(ToolError::MissingRequiredArg {
            tool: tool.to_string(),
            arg: key.to_string(),
        }),
    }
}

#[derive(Default)]
pub struct ToolDispatcher {
    tools: std::collections::HashMap<String, Box<dyn Tool>>,
}

impl ToolDispatcher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) -> Result<(), ToolError> {
        let name = tool.name().to_string();
        if self.tools.contains_key(&name) {
            return Err(ToolError::DuplicateName(name));
        }
        self.tools.insert(name, tool);
        Ok(())
    }

    pub fn get(&self, name: &str) -> Result<&dyn Tool, ToolError> {
        self.tools
            .get(name)
            .map(|b| b.as_ref())
            .ok_or_else(|| ToolError::UnknownTool(name.to_string()))
    }

    pub fn dispatch(
        &self,
        name: &str,
        args: &serde_json::Value,
        ctx: &ToolCtx,
    ) -> Result<serde_json::Value, ToolError> {
        Ok(self.get(name)?.invoke(args, ctx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;
    use tempfile::NamedTempFile;

    struct StubTool {
        name: &'static str,
        response: serde_json::Value,
    }

    impl Tool for StubTool {
        fn name(&self) -> &str {
            self.name
        }
        fn approval_preview(&self, _args: &serde_json::Value) -> ApprovalPreview {
            ApprovalPreview {
                description: format!("stub: {}", self.name),
            }
        }
        fn invoke(&self, _args: &serde_json::Value, _ctx: &ToolCtx) -> serde_json::Value {
            self.response.clone()
        }
    }

    fn empty_ctx() -> ToolCtx {
        ToolCtx::default()
    }

    #[test]
    fn register_and_dispatch_returns_tool_result() {
        let mut d = ToolDispatcher::new();
        d.register(Box::new(StubTool {
            name: "noop",
            response: json!({"ok": true}),
        }))
        .unwrap();

        let result = d.dispatch("noop", &json!({}), &empty_ctx()).unwrap();
        assert_eq!(result, json!({"ok": true}));
    }

    #[test]
    fn duplicate_registration_returns_error() {
        let mut d = ToolDispatcher::new();
        d.register(Box::new(StubTool {
            name: "noop",
            response: json!({}),
        }))
        .unwrap();

        let err = d
            .register(Box::new(StubTool {
                name: "noop",
                response: json!({}),
            }))
            .unwrap_err();
        assert_eq!(err, ToolError::DuplicateName("noop".to_string()));
    }

    #[test]
    fn dispatch_unknown_tool_returns_error() {
        let d = ToolDispatcher::new();
        let err = d.dispatch("nope", &json!({}), &empty_ctx()).unwrap_err();
        assert_eq!(err, ToolError::UnknownTool("nope".to_string()));
    }

    // ---- F-074: shared `get_required_str` helper ----

    #[test]
    fn get_required_str_returns_string_when_present() {
        let v = json!({ "path": "/tmp/x" });
        assert_eq!(get_required_str(&v, "fs.read", "path").unwrap(), "/tmp/x");
    }

    #[test]
    fn get_required_str_accepts_empty_string() {
        // F-074: empty is intentionally allowed so `fs.write` can truncate
        // a file via `{"content": ""}`. Tools that need stricter checks
        // (e.g. `shell.exec` rejecting `""` for `command`) layer the
        // empty-guard on top of this helper.
        let v = json!({ "content": "" });
        assert_eq!(get_required_str(&v, "fs.write", "content").unwrap(), "");
    }

    #[test]
    fn get_required_str_rejects_missing_key_with_unified_shape() {
        let v = json!({});
        let err = get_required_str(&v, "fs.read", "path").unwrap_err();
        assert_eq!(
            err,
            ToolError::MissingRequiredArg {
                tool: "fs.read".to_string(),
                arg: "path".to_string()
            }
        );
        assert_eq!(
            err.to_string(),
            "tool.fs.read: missing required parameter 'path'"
        );
    }

    #[test]
    fn get_required_str_rejects_non_string_value() {
        let v = json!({ "path": 42 });
        let err = get_required_str(&v, "fs.read", "path").unwrap_err();
        assert_eq!(
            err,
            ToolError::MissingRequiredArg {
                tool: "fs.read".to_string(),
                arg: "path".to_string()
            }
        );
    }

    // ---- F-074: each tool surfaces the unified missing-arg error from
    // `invoke` rather than coercing missing required args to "" and producing
    // a confusing downstream error from `forge_fs` / sandbox. ----

    #[test]
    fn fs_read_invoke_errors_explicitly_on_missing_path() {
        let mut d = ToolDispatcher::new();
        d.register(Box::new(FsReadTool)).unwrap();
        let result = d.dispatch("fs.read", &json!({}), &empty_ctx()).unwrap();
        assert_eq!(
            result["error"].as_str(),
            Some("tool.fs.read: missing required parameter 'path'"),
            "result was: {result}"
        );
    }

    #[test]
    fn fs_write_invoke_errors_explicitly_on_missing_path() {
        let mut d = ToolDispatcher::new();
        d.register(Box::new(FsWriteTool)).unwrap();
        let result = d
            .dispatch("fs.write", &json!({ "content": "hi" }), &empty_ctx())
            .unwrap();
        assert_eq!(
            result["error"].as_str(),
            Some("tool.fs.write: missing required parameter 'path'"),
            "result was: {result}"
        );
    }

    #[test]
    fn fs_write_invoke_errors_explicitly_on_missing_content() {
        let mut d = ToolDispatcher::new();
        d.register(Box::new(FsWriteTool)).unwrap();
        let result = d
            .dispatch(
                "fs.write",
                &json!({ "path": "/tmp/forge-f074-noop" }),
                &empty_ctx(),
            )
            .unwrap();
        assert_eq!(
            result["error"].as_str(),
            Some("tool.fs.write: missing required parameter 'content'"),
            "result was: {result}"
        );
    }

    #[test]
    fn fs_write_invoke_allows_empty_content_to_truncate() {
        // Empty content must remain a valid request — only missing keys
        // should error. Otherwise the F-074 helper would silently break the
        // legitimate "create empty file" / "truncate" use case.
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("empty.txt");
        std::fs::write(&target, "old contents").unwrap();
        let canonical_parent = std::fs::canonicalize(dir.path()).unwrap();
        let allowed = format!("{}/**", canonical_parent.to_str().unwrap());

        let mut d = ToolDispatcher::new();
        d.register(Box::new(FsWriteTool)).unwrap();
        let ctx = ToolCtx {
            allowed_paths: vec![allowed],
            ..ToolCtx::default()
        };
        let result = d
            .dispatch(
                "fs.write",
                &json!({ "path": target.to_str().unwrap(), "content": "" }),
                &ctx,
            )
            .unwrap();
        assert_eq!(result["ok"].as_bool(), Some(true), "result: {result}");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "");
    }

    #[test]
    fn fs_edit_invoke_errors_explicitly_on_missing_path() {
        let mut d = ToolDispatcher::new();
        d.register(Box::new(FsEditTool)).unwrap();
        let result = d
            .dispatch("fs.edit", &json!({ "patch": "@@ -1 +1 @@" }), &empty_ctx())
            .unwrap();
        assert_eq!(
            result["error"].as_str(),
            Some("tool.fs.edit: missing required parameter 'path'"),
            "result was: {result}"
        );
    }

    #[test]
    fn fs_edit_invoke_errors_explicitly_on_missing_patch() {
        let mut d = ToolDispatcher::new();
        d.register(Box::new(FsEditTool)).unwrap();
        let result = d
            .dispatch(
                "fs.edit",
                &json!({ "path": "/tmp/forge-f074-noop" }),
                &empty_ctx(),
            )
            .unwrap();
        assert_eq!(
            result["error"].as_str(),
            Some("tool.fs.edit: missing required parameter 'patch'"),
            "result was: {result}"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn shell_exec_invoke_errors_explicitly_on_missing_command() {
        let dir = tempfile::tempdir().unwrap();
        let mut d = ToolDispatcher::new();
        d.register(Box::new(ShellExecTool)).unwrap();
        let ctx = ToolCtx {
            allowed_paths: vec![],
            workspace_root: Some(dir.path().to_path_buf()),
            child_registry: Some(crate::sandbox::ChildRegistry::new()),
        };
        let result = d.dispatch("shell.exec", &json!({}), &ctx).unwrap();
        assert_eq!(
            result["error"].as_str(),
            Some("tool.shell.exec: missing required parameter 'command'"),
            "result was: {result}"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn shell_exec_invoke_errors_explicitly_on_empty_command() {
        // shell.exec layers an empty-guard on top of `get_required_str`
        // because spawning `""` is meaningless. The error shape stays
        // unified ("tool.X: missing required parameter 'Y'") so the IPC
        // surface is consistent across all four tools.
        let dir = tempfile::tempdir().unwrap();
        let mut d = ToolDispatcher::new();
        d.register(Box::new(ShellExecTool)).unwrap();
        let ctx = ToolCtx {
            allowed_paths: vec![],
            workspace_root: Some(dir.path().to_path_buf()),
            child_registry: Some(crate::sandbox::ChildRegistry::new()),
        };
        let result = d
            .dispatch("shell.exec", &json!({ "command": "" }), &ctx)
            .unwrap();
        assert_eq!(
            result["error"].as_str(),
            Some("tool.shell.exec: missing required parameter 'command'"),
            "result was: {result}"
        );
    }

    #[test]
    fn fs_write_dispatch_writes_file_and_previews_diff() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("out.txt");
        let canonical_parent = std::fs::canonicalize(dir.path()).unwrap();
        let allowed = format!("{}/**", canonical_parent.to_str().unwrap());

        let mut d = ToolDispatcher::new();
        d.register(Box::new(FsWriteTool)).unwrap();

        let ctx = ToolCtx {
            allowed_paths: vec![allowed],
            ..ToolCtx::default()
        };
        let result = d
            .dispatch(
                "fs.write",
                &json!({"path": target.to_str().unwrap(), "content": "hi"}),
                &ctx,
            )
            .unwrap();
        assert_eq!(result["ok"].as_bool(), Some(true));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "hi");

        let preview = d
            .get("fs.write")
            .unwrap()
            .approval_preview(&json!({"path": target.to_str().unwrap(), "content": "hi"}));
        assert!(preview.description.contains("Write file"));
    }

    #[test]
    fn fs_edit_dispatch_applies_patch_and_previews_diff() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("src.txt");
        std::fs::write(&target, "alpha\nbeta\n").unwrap();
        let canonical_parent = std::fs::canonicalize(dir.path()).unwrap();
        let allowed = format!("{}/**", canonical_parent.to_str().unwrap());

        let patch = similar::TextDiff::from_lines("alpha\nbeta\n", "alpha\nBETA\n")
            .unified_diff()
            .to_string();

        let mut d = ToolDispatcher::new();
        d.register(Box::new(FsEditTool)).unwrap();

        let ctx = ToolCtx {
            allowed_paths: vec![allowed],
            ..ToolCtx::default()
        };
        let result = d
            .dispatch(
                "fs.edit",
                &json!({"path": target.to_str().unwrap(), "patch": patch}),
                &ctx,
            )
            .unwrap();
        assert_eq!(result["ok"].as_bool(), Some(true));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "alpha\nBETA\n");

        let preview = d
            .get("fs.edit")
            .unwrap()
            .approval_preview(&json!({"path": target.to_str().unwrap(), "patch": patch}));
        assert!(preview.description.contains("Edit file"));
    }

    #[test]
    fn fs_read_dispatch_returns_content_bytes_sha256() {
        let mut file = NamedTempFile::new().unwrap();
        let body = "hello dispatcher";
        file.write_all(body.as_bytes()).unwrap();
        let path = file.path().to_str().unwrap().to_string();
        let canonical = std::fs::canonicalize(&path).unwrap();
        let allowed = canonical.to_str().unwrap().to_string();

        let mut d = ToolDispatcher::new();
        d.register(Box::new(FsReadTool)).unwrap();

        let ctx = ToolCtx {
            allowed_paths: vec![allowed],
            ..ToolCtx::default()
        };
        let result = d.dispatch("fs.read", &json!({"path": path}), &ctx).unwrap();

        assert_eq!(result["content"].as_str().unwrap(), body);
        assert_eq!(result["bytes"].as_u64().unwrap(), body.len() as u64);
        assert_eq!(result["sha256"].as_str().unwrap().len(), 64);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn shell_exec_dispatch_runs_command_and_captures_stdout() {
        let dir = tempfile::tempdir().unwrap();

        let mut d = ToolDispatcher::new();
        d.register(Box::new(ShellExecTool)).unwrap();

        let ctx = ToolCtx {
            allowed_paths: vec![],
            workspace_root: Some(dir.path().to_path_buf()),
            child_registry: Some(crate::sandbox::ChildRegistry::new()),
        };
        let result = d
            .dispatch(
                "shell.exec",
                &json!({"command": "/bin/sh", "args": ["-c", "echo hello-sandbox"], "timeout_ms": 5000}),
                &ctx,
            )
            .unwrap();

        assert_eq!(result["exit_code"].as_i64(), Some(0), "result: {result}");
        assert_eq!(
            result["stdout"].as_str().unwrap().trim(),
            "hello-sandbox",
            "result: {result}"
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shell_exec_dispatch_works_inside_outer_tokio_runtime() {
        // Regression: Tool::invoke is called from async context in run_turn.
        // The implementation must not panic attempting to nest runtimes.
        let dir = tempfile::tempdir().unwrap();

        let mut d = ToolDispatcher::new();
        d.register(Box::new(ShellExecTool)).unwrap();

        let ctx = ToolCtx {
            allowed_paths: vec![],
            workspace_root: Some(dir.path().to_path_buf()),
            child_registry: Some(crate::sandbox::ChildRegistry::new()),
        };

        // Run dispatch on a blocking task so we're inside the runtime but
        // block_in_place is permitted.
        let result = d
            .dispatch(
                "shell.exec",
                &json!({"command": "/bin/sh", "args": ["-c", "echo from-async"], "timeout_ms": 5000}),
                &ctx,
            )
            .unwrap();

        assert_eq!(result["exit_code"].as_i64(), Some(0), "result: {result}");
        assert_eq!(result["stdout"].as_str().unwrap().trim(), "from-async");
    }

    #[cfg(target_os = "linux")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shell_exec_timeout_kills_sandboxed_grandchild() {
        // F-047 / H6: after `shell.exec` returns a timeout error, the
        // sandboxed process group (including detached grandchildren) must
        // be dead. Before the fix, `run_linux` called `.into_child()` which
        // detached the `SandboxedChild` from the registry and disabled the
        // Drop-based `killpg`, so the grandchild leaked past both the
        // tool call and daemon shutdown.

        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("grandchild.pid");

        let mut d = ToolDispatcher::new();
        d.register(Box::new(ShellExecTool)).unwrap();

        let registry = crate::sandbox::ChildRegistry::new();
        let ctx = ToolCtx {
            allowed_paths: vec![],
            workspace_root: Some(dir.path().to_path_buf()),
            child_registry: Some(registry.clone()),
        };

        // Background a long sleep, write its pid, detach its stdio from
        // the parent shell so that when the shell exits the wait_fut may
        // complete but the grandchild stays alive. The parent shell needs
        // to keep running long enough for the timeout to fire, so we also
        // sleep in the foreground.
        let script = format!(
            "sleep 30 </dev/null >/dev/null 2>&1 & echo $! > {}; sleep 30",
            pid_file.display()
        );

        let ctx_clone_args = serde_json::json!({
            "command": "/bin/sh",
            "args": ["-c", script],
            "timeout_ms": 200,
        });

        let result = tokio::task::spawn_blocking(move || {
            d.dispatch("shell.exec", &ctx_clone_args, &ctx).unwrap()
        })
        .await
        .unwrap();

        assert!(
            result.get("error").is_some(),
            "expected timeout error, got: {result}"
        );

        // Read grandchild pid written by the script.
        let mut grandchild_pid: Option<i32> = None;
        for _ in 0..50 {
            if let Ok(s) = std::fs::read_to_string(&pid_file) {
                if let Ok(pid) = s.trim().parse::<i32>() {
                    grandchild_pid = Some(pid);
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let grandchild_pid = grandchild_pid.expect("script never wrote grandchild pid");

        // Poll for the grandchild to be reaped — killpg delivery plus shell
        // teardown can take a few ms on loaded CI.
        let mut alive = true;
        for _ in 0..100 {
            let rc = unsafe { libc::kill(grandchild_pid, 0) };
            if rc != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
                alive = false;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(
            !alive,
            "grandchild {grandchild_pid} survived shell.exec timeout \
             (sandbox escape — pgid was not killed)"
        );

        // Secondary assertion: the registry entry was cleared. If Drop
        // on `SandboxedChild` ran, `registry.remove(pgid)` executed.
        assert!(
            registry.is_empty(),
            "ChildRegistry still tracks pgid after timeout — Drop did not run"
        );
    }

    #[test]
    fn shell_exec_preview_includes_cwd_when_provided() {
        let tool = ShellExecTool;
        let preview = tool.approval_preview(&json!({
            "command": "ls",
            "args": ["."],
            "cwd": "/tmp/somewhere-specific"
        }));
        assert!(
            preview.description.contains("/tmp/somewhere-specific"),
            "cwd missing from preview: {}",
            preview.description
        );
        assert!(
            preview.description.contains("ls"),
            "command missing from preview: {}",
            preview.description
        );
    }

    #[test]
    fn shell_exec_preview_omits_cwd_when_absent() {
        let tool = ShellExecTool;
        let preview = tool.approval_preview(&json!({
            "command": "ls",
            "args": ["."]
        }));
        assert!(
            !preview.description.to_lowercase().contains("cwd"),
            "cwd should not appear when absent: {}",
            preview.description
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn shell_exec_dispatch_rejects_cwd_outside_workspace() {
        let dir = tempfile::tempdir().unwrap();

        let mut d = ToolDispatcher::new();
        d.register(Box::new(ShellExecTool)).unwrap();

        let ctx = ToolCtx {
            allowed_paths: vec![],
            workspace_root: Some(dir.path().to_path_buf()),
            child_registry: Some(crate::sandbox::ChildRegistry::new()),
        };
        let result = d
            .dispatch(
                "shell.exec",
                &json!({
                    "command": "/bin/sh",
                    "args": ["-c", "echo should-not-run"],
                    "cwd": "/etc",
                    "timeout_ms": 5000
                }),
                &ctx,
            )
            .unwrap();

        assert!(
            result.get("error").is_some(),
            "expected error when cwd is outside workspace; got: {result}"
        );
        assert!(
            result.get("exit_code").is_none(),
            "command must not execute when cwd is rejected; got: {result}"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn shell_exec_dispatch_accepts_cwd_inside_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();

        let mut d = ToolDispatcher::new();
        d.register(Box::new(ShellExecTool)).unwrap();

        let ctx = ToolCtx {
            allowed_paths: vec![],
            workspace_root: Some(dir.path().to_path_buf()),
            child_registry: Some(crate::sandbox::ChildRegistry::new()),
        };
        let result = d
            .dispatch(
                "shell.exec",
                &json!({
                    "command": "/bin/sh",
                    "args": ["-c", "pwd"],
                    "cwd": sub.to_str().unwrap(),
                    "timeout_ms": 5000
                }),
                &ctx,
            )
            .unwrap();

        assert_eq!(result["exit_code"].as_i64(), Some(0), "result: {result}");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn shell_exec_dispatch_rejects_cwd_via_symlink_escape() {
        // Symlink inside the workspace pointing outside must be rejected:
        // canonicalize() resolves the symlink, and the post-canonical
        // prefix check catches the escape.
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("escape");
        std::os::unix::fs::symlink("/etc", &link).unwrap();

        let mut d = ToolDispatcher::new();
        d.register(Box::new(ShellExecTool)).unwrap();

        let ctx = ToolCtx {
            allowed_paths: vec![],
            workspace_root: Some(dir.path().to_path_buf()),
            child_registry: Some(crate::sandbox::ChildRegistry::new()),
        };
        let result = d
            .dispatch(
                "shell.exec",
                &json!({
                    "command": "/bin/sh",
                    "args": ["-c", "echo should-not-run"],
                    "cwd": link.to_str().unwrap(),
                    "timeout_ms": 5000
                }),
                &ctx,
            )
            .unwrap();

        assert!(
            result.get("error").is_some(),
            "symlink escape must be rejected; got: {result}"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn shell_exec_clamps_timeout_ms_to_documented_ceiling() {
        // Regression for F-066: a provider may pass an arbitrarily large
        // timeout_ms (up to u64::MAX). Without the clamp, tokio::time::timeout
        // receives a far-future deadline and a shell backgrounding sleeps can
        // hold the tool-call future open indefinitely. With the clamp, the
        // deadline is capped at MAX_TIMEOUT_MS (10 minutes) so the tool call
        // always resolves within the ceiling. We drive a fast-exiting command
        // here so the test runs quickly — the assertion is that the dispatch
        // returns successfully (proving u64::MAX does not overflow or hang)
        // and completes well under the ceiling.
        let dir = tempfile::tempdir().unwrap();

        let mut d = ToolDispatcher::new();
        d.register(Box::new(ShellExecTool)).unwrap();

        let ctx = ToolCtx {
            allowed_paths: vec![],
            workspace_root: Some(dir.path().to_path_buf()),
            child_registry: Some(crate::sandbox::ChildRegistry::new()),
        };

        let started = std::time::Instant::now();
        let result = d
            .dispatch(
                "shell.exec",
                &json!({"command": "/bin/true", "timeout_ms": u64::MAX}),
                &ctx,
            )
            .unwrap();
        let elapsed = started.elapsed();

        assert_eq!(result["exit_code"].as_i64(), Some(0), "result: {result}");
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "u64::MAX timeout caused dispatch to hang for {elapsed:?}"
        );
        // Compile-time guard: ceiling exists and stays within the documented
        // 10-minute budget.
        const _: () = assert!(shell_exec::MAX_TIMEOUT_MS <= 10 * 60 * 1000);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn shell_exec_dispatch_clears_daemon_env_by_default() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("FORGE_SHELL_EXEC_CANARY", "nope");

        let mut d = ToolDispatcher::new();
        d.register(Box::new(ShellExecTool)).unwrap();

        let ctx = ToolCtx {
            allowed_paths: vec![],
            workspace_root: Some(dir.path().to_path_buf()),
            child_registry: Some(crate::sandbox::ChildRegistry::new()),
        };
        let result = d
            .dispatch(
                "shell.exec",
                &json!({"command": "/usr/bin/env", "timeout_ms": 5000}),
                &ctx,
            )
            .unwrap();
        std::env::remove_var("FORGE_SHELL_EXEC_CANARY");

        let stdout = result["stdout"].as_str().unwrap_or_default();
        assert!(
            !stdout.contains("FORGE_SHELL_EXEC_CANARY"),
            "daemon env leaked into shell.exec:\n{stdout}"
        );
    }
}
