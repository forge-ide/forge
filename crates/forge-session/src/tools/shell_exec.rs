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

    // shell.exec is invoked from the sync Tool::invoke path; run a scoped
    // tokio runtime for the single spawn/wait.
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => return serde_json::json!({ "error": format!("tokio runtime: {e}") }),
    };

    rt.block_on(async move {
        let sandboxed = match sb.spawn() {
            Ok(c) => c,
            Err(e) => return serde_json::json!({ "error": format!("spawn: {e}") }),
        };
        let mut child = sandboxed.into_child();

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

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

        let wait_fut = async move { child.wait().await };
        let combined = async move { tokio::join!(wait_fut, stdout_fut, stderr_fut) };

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
    })
}
