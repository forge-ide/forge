//! `shell.exec` tool stub: runs a command through the Level-1 sandbox and
//! captures stdout / stderr / exit code. Streaming support is deferred.
//!
//! Args (JSON):
//! ```json
//! {
//!   "command": "/usr/bin/env",      // required
//!   "args": ["FOO"],                // optional
//!   "cwd": "/path/to/workspace",    // optional; defaults to ctx.workspace_root
//!   "env": { "FOO": "bar" },        // optional caller-scoped env allow-list
//!   "timeout_ms": 30000             // optional wall-clock guard
//! }
//! ```
//!
//! Result shape: `{ stdout, stderr, exit_code, signal? }` or `{ error }`.

use super::{Tool, ToolCtx};
#[cfg(target_os = "linux")]
use crate::sandbox::SandboxedCommand;
use forge_core::ApprovalPreview;
#[cfg(target_os = "linux")]
use std::time::Duration;

pub struct ShellExecTool;

impl ShellExecTool {
    pub const NAME: &'static str = "shell.exec";
}

impl Tool for ShellExecTool {
    fn name(&self) -> &str {
        Self::NAME
    }

    fn approval_preview(&self, args: &serde_json::Value) -> ApprovalPreview {
        let command = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
        let argv = args
            .get("args")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        ApprovalPreview {
            description: if argv.is_empty() {
                format!("Run command: {command}")
            } else {
                format!("Run command: {command} {argv}")
            },
        }
    }

    fn invoke(&self, args: &serde_json::Value, ctx: &ToolCtx) -> serde_json::Value {
        #[cfg(target_os = "linux")]
        {
            run_linux(args, ctx)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (args, ctx);
            serde_json::json!({
                "error": "shell.exec requires Linux (process isolation not yet implemented on this platform)"
            })
        }
    }
}

#[cfg(target_os = "linux")]
fn run_linux(args: &serde_json::Value, ctx: &ToolCtx) -> serde_json::Value {
    let command = match args.get("command").and_then(|v| v.as_str()) {
        Some(c) if !c.is_empty() => c,
        _ => return serde_json::json!({ "error": "shell.exec: missing 'command'" }),
    };

    let argv: Vec<String> = args
        .get("args")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();

    let cwd = args
        .get("cwd")
        .and_then(|v| v.as_str())
        .map(std::path::PathBuf::from)
        .or_else(|| ctx.workspace_root.clone())
        .unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
        });

    let timeout_ms = args
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(30_000);

    let mut sb = SandboxedCommand::new(command, &cwd);
    sb.command_mut().args(&argv);
    sb.command_mut().stdout(std::process::Stdio::piped());
    sb.command_mut().stderr(std::process::Stdio::piped());
    sb.command_mut().stdin(std::process::Stdio::null());

    if let Some(env_obj) = args.get("env").and_then(|v| v.as_object()) {
        for (k, v) in env_obj {
            if let Some(val) = v.as_str() {
                sb.allow_env(k, val);
            }
        }
    }

    if let Some(registry) = &ctx.child_registry {
        sb.with_registry(registry.clone());
    }

    // `Tool::invoke` is synchronous but is called from inside an async
    // context in `run_turn`. If we are on a multi-threaded runtime we can
    // block_in_place; otherwise fall back to a fresh current-thread runtime.
    //
    // F-047 / H6: the `SandboxedChild` wrapper MUST stay live across the
    // timeout. Calling `.into_child()` here would set `released = true` and
    // remove the pgid from the session's `ChildRegistry`, so a timeout (or
    // task cancellation / panic) drop of the `tokio::process::Child` alone
    // would orphan the process group — the whole point of Level-1 isolation
    // defeated. Keeping `sandboxed` in scope guarantees `Drop::killpg` runs
    // on every exit path and the registry entry is cleaned up.
    let fut = async move {
        let mut sandboxed = match sb.spawn() {
            Ok(c) => c,
            Err(e) => return serde_json::json!({ "error": format!("spawn: {e}") }),
        };

        // Take the stdout / stderr pipes by value so the concurrent reader
        // futures can own them; we only need `&mut Child` for `wait()`.
        let stdout = sandboxed.as_child_mut().stdout.take();
        let stderr = sandboxed.as_child_mut().stderr.take();

        let stdout_fut = async move {
            match stdout {
                Some(mut s) => {
                    use tokio::io::AsyncReadExt;
                    let mut buf = String::new();
                    let _ = s.read_to_string(&mut buf).await;
                    buf
                }
                None => String::new(),
            }
        };
        let stderr_fut = async move {
            match stderr {
                Some(mut s) => {
                    use tokio::io::AsyncReadExt;
                    let mut buf = String::new();
                    let _ = s.read_to_string(&mut buf).await;
                    buf
                }
                None => String::new(),
            }
        };

        // Run wait + stdout + stderr concurrently so the child cannot block
        // on full pipe buffers while we wait on exit. `sandboxed` stays
        // borrowed by `as_child_mut()` only for the duration of the join.
        let combined =
            async { tokio::join!(sandboxed.as_child_mut().wait(), stdout_fut, stderr_fut,) };

        match tokio::time::timeout(Duration::from_millis(timeout_ms), combined).await {
            Ok((Ok(status), stdout, stderr)) => {
                use std::os::unix::process::ExitStatusExt;
                let mut result = serde_json::json!({
                    "stdout": stdout,
                    "stderr": stderr,
                    "exit_code": status.code(),
                });
                if let Some(sig) = status.signal() {
                    result["signal"] = serde_json::json!(sig);
                }
                result
            }
            Ok((Err(e), _, _)) => serde_json::json!({ "error": format!("wait: {e}") }),
            Err(_) => serde_json::json!({ "error": format!("timeout after {timeout_ms}ms") }),
        }
        // `sandboxed` drops here — on both success (drop is idempotent;
        // killpg on an already-reaped pgid returns ESRCH) and on timeout
        // (drop sends SIGKILL to the pgid and removes it from the registry).
    };

    match tokio::runtime::Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| handle.block_on(fut)),
        Err(_) => {
            // No outer runtime (e.g. sync unit tests) — build a throwaway one.
            match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt.block_on(fut),
                Err(e) => serde_json::json!({ "error": format!("tokio runtime: {e}") }),
            }
        }
    }
}
