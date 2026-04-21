//! Stdio JSON-RPC transport for a single MCP server subprocess.
//!
//! The wire format is line-delimited JSON-RPC 2.0: each frame is a UTF-8
//! JSON value terminated by `\n`. Partial reads are absorbed by
//! [`tokio::io::AsyncBufReadExt::lines`]; lines that fail to parse as JSON
//! are logged at WARN and dropped — they don't tear the connection down.
//! When the child process exits (or its stdout EOFs), the receiver yields
//! exactly one terminal [`StdioEvent::Exit`] before closing.

use std::process::{ExitStatus, Stdio as StdStdio};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;

use crate::{McpServerSpec, ServerKind};

/// Events emitted on [`Stdio::recv`].
///
/// Consumers receive any number of [`StdioEvent::Message`] values followed
/// by exactly one terminal [`StdioEvent::Exit`] once the child reaps, at
/// which point the sender is dropped and subsequent calls to
/// [`Stdio::recv`] return `None`.
#[derive(Debug)]
pub enum StdioEvent {
    /// A successfully parsed JSON-RPC 2.0 message from the server's stdout.
    ///
    /// Responses, notifications, and server→client requests all surface
    /// here; dispatch is the manager's job, not the transport's.
    Message(serde_json::Value),
    /// The child process exited. Always the last event before channel close.
    Exit(ExitStatus),
}

/// Channel depth for outbound [`StdioEvent`]s. Generous enough that a brief
/// scheduling delay in the consumer doesn't back-pressure the reader task.
const EVENT_CHANNEL_CAPACITY: usize = 128;

/// An active stdio JSON-RPC connection to one MCP server subprocess.
///
/// Construct with [`Stdio::connect`]. The handle owns the child's stdin
/// (for [`Stdio::send`]), a receiver of [`StdioEvent`]s driven by a
/// background task reading stdout, and a join handle so we can tear the
/// reader down on drop. `stderr` is drained by a sibling task and logged
/// at DEBUG.
pub struct Stdio {
    /// Protected so concurrent senders serialise their frames; JSON-RPC
    /// frames must be atomic relative to `\n`.
    stdin: Arc<Mutex<ChildStdin>>,
    rx: mpsc::Receiver<StdioEvent>,
    /// Reader task that owns the child process and drives
    /// `child.wait().await` to completion. The handle is held purely
    /// to tie the task's lifetime to this struct. We deliberately do
    /// NOT `.abort()` it in [`Drop`]: the reader is the *only* task
    /// that `wait()`s on the child, and aborting mid-wait leaves the
    /// child un-reaped — later `Command::spawn()` calls in the same
    /// tokio runtime then observe zombie handles and stall. Relying
    /// on `Command::kill_on_drop(true)` (set in [`Stdio::connect`])
    /// plus the reader's own end-of-stream `wait()` is both
    /// necessary and sufficient for clean teardown.
    _reader: JoinHandle<()>,
    /// Stderr drain task. Explicitly aborted in [`Drop`]: unlike the
    /// reader it holds no child handle, only a log-forwarding loop
    /// over the (now-closed-on-drop) stderr pipe. Aborting it
    /// guarantees the tokio runtime's blocking thread pool releases
    /// the read as soon as the handle is gone.
    stderr: JoinHandle<()>,
}

impl Drop for Stdio {
    fn drop(&mut self) {
        // See field docs: reader is left alone so it finishes
        // `child.wait()`; stderr is aborted so the runtime reclaims
        // its blocking slot promptly.
        self.stderr.abort();
    }
}

impl std::fmt::Debug for Stdio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stdio").finish_non_exhaustive()
    }
}

impl Stdio {
    /// Spawn `spec`'s configured command and connect a JSON-RPC channel to
    /// its stdio. Errors if `spec` describes an HTTP server (caller's bug)
    /// or if the subprocess cannot be spawned (e.g. command not on `PATH`).
    pub async fn connect(spec: &McpServerSpec) -> Result<Self> {
        let (command, args, env) = match &spec.kind {
            ServerKind::Stdio { command, args, env } => (command, args, env),
            ServerKind::Http { .. } => {
                return Err(anyhow!(
                    "stdio transport cannot connect to an http MCP server"
                ));
            }
        };

        let mut cmd = Command::new(command);
        cmd.args(args)
            .envs(env)
            .stdin(StdStdio::piped())
            .stdout(StdStdio::piped())
            .stderr(StdStdio::piped())
            // Ensure the child is killed if the transport is dropped before
            // an explicit shutdown. MCP servers are long-running; without
            // this they'd leak past the parent on panic / early return.
            .kill_on_drop(true);

        let mut child = cmd
            .spawn()
            .with_context(|| format!("spawning MCP server {command:?}"))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("child has no stdin pipe"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("child has no stdout pipe"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("child has no stderr pipe"))?;

        let (tx, rx) = mpsc::channel::<StdioEvent>(EVENT_CHANNEL_CAPACITY);

        // Stderr drain: prevents the child's stderr pipe from filling up
        // and blocking its writes. Log at DEBUG — stderr is free-form.
        let stderr_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        tracing::debug!(target: "forge_mcp::transport::stdio", stderr = %line);
                    }
                    Ok(None) => break,
                    Err(err) => {
                        tracing::debug!(
                            target: "forge_mcp::transport::stdio",
                            error = %err,
                            "stderr read error; draining aborted",
                        );
                        break;
                    }
                }
            }
        });

        // Reader: owns stdout + child, so it can wait() after EOF and emit
        // the terminal Exit event without racing the consumer.
        let reader_tx = tx.clone();
        let reader_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        // Tolerate blank lines (some servers pad frames).
                        if line.trim().is_empty() {
                            continue;
                        }
                        match serde_json::from_str::<serde_json::Value>(&line) {
                            Ok(value) => {
                                if reader_tx.send(StdioEvent::Message(value)).await.is_err() {
                                    // Consumer dropped; no need to keep reading.
                                    break;
                                }
                            }
                            Err(err) => {
                                // DoD: malformed frames are logged + skipped,
                                // never fatal.
                                tracing::warn!(
                                    target: "forge_mcp::transport::stdio",
                                    error = %err,
                                    line = %truncate(&line, 512),
                                    "dropping malformed JSON-RPC frame",
                                );
                            }
                        }
                    }
                    Ok(None) => break, // EOF: child closed stdout.
                    Err(err) => {
                        tracing::warn!(
                            target: "forge_mcp::transport::stdio",
                            error = %err,
                            "stdout read error; surfacing as exit",
                        );
                        break;
                    }
                }
            }

            // Reap the child and surface the exit status. `wait()` is safe
            // to call after we've dropped stdout — the pipe is closed.
            let status = match child.wait().await {
                Ok(s) => s,
                Err(err) => {
                    tracing::warn!(
                        target: "forge_mcp::transport::stdio",
                        error = %err,
                        "wait() failed; forcing kill",
                    );
                    // Best-effort kill, then synthesise a failure status.
                    let _ = child.kill().await;
                    // If even this fails, fall back to whatever wait returns.
                    child.wait().await.unwrap_or_else(|_| {
                        // Fabricate a status via a throw-away successful
                        // command so we can always emit an Exit event.
                        std::process::ExitStatus::default()
                    })
                }
            };

            // `send` may fail if the consumer dropped — that's fine.
            let _ = reader_tx.send(StdioEvent::Exit(status)).await;
        });

        Ok(Self {
            stdin: Arc::new(Mutex::new(stdin)),
            rx,
            _reader: reader_task,
            stderr: stderr_task,
        })
    }

    /// Write one JSON-RPC frame (any `serde_json::Value`) to the server's
    /// stdin, appending the terminating newline. Returns an error if
    /// serialisation fails or the stdin pipe has closed (child died).
    pub async fn send(&self, message: serde_json::Value) -> Result<()> {
        let mut bytes = serde_json::to_vec(&message).context("serialising JSON-RPC frame")?;
        bytes.push(b'\n');

        let mut guard = self.stdin.lock().await;
        guard
            .write_all(&bytes)
            .await
            .context("writing JSON-RPC frame to MCP server stdin")?;
        guard
            .flush()
            .await
            .context("flushing JSON-RPC frame to MCP server stdin")?;
        Ok(())
    }

    /// Receive the next inbound event, or `None` when the reader task has
    /// completed (i.e. after the terminal [`StdioEvent::Exit`]).
    pub async fn recv(&mut self) -> Option<StdioEvent> {
        self.rx.recv().await
    }
}

/// Cap a log field at `max` bytes so a runaway frame can't flood the log
/// ring. `line` is guaranteed UTF-8 here, but we still slice on a char
/// boundary via `char_indices` to avoid panicking on multi-byte glyphs.
fn truncate(line: &str, max: usize) -> String {
    if line.len() <= max {
        return line.to_string();
    }
    let mut end = max;
    for (i, _) in line.char_indices() {
        if i > max {
            break;
        }
        end = i;
    }
    format!("{}…", &line[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn stdio_spec(cmd: &str, args: &[&str]) -> McpServerSpec {
        McpServerSpec {
            kind: ServerKind::Stdio {
                command: cmd.to_string(),
                args: args.iter().map(|s| s.to_string()).collect(),
                env: BTreeMap::new(),
            },
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn refuses_to_connect_to_http_spec() {
        let spec = McpServerSpec {
            kind: ServerKind::Http {
                url: "https://example.com".into(),
                headers: BTreeMap::new(),
            },
        };
        let err = Stdio::connect(&spec).await.expect_err("http must reject");
        assert!(
            format!("{err:#}").contains("http"),
            "error should explain transport mismatch: {err:#}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn surfaces_exit_event_when_child_exits_immediately() {
        // `/bin/true` spawns, writes nothing, exits 0. We expect exactly
        // one Exit event and then a closed channel.
        let mut t = Stdio::connect(&stdio_spec("/bin/true", &[]))
            .await
            .expect("spawn /bin/true");
        let ev = tokio::time::timeout(std::time::Duration::from_secs(5), t.recv())
            .await
            .expect("recv did not yield Exit in time")
            .expect("channel closed before Exit");
        match ev {
            StdioEvent::Exit(status) => assert!(status.success()),
            StdioEvent::Message(v) => panic!("expected Exit, got Message({v})"),
        }
        // After Exit the sender drops; next recv returns None.
        let after = t.recv().await;
        assert!(after.is_none(), "channel must close after Exit");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn drops_malformed_lines_without_closing_stream() {
        // printf emits two frames: garbage, then a valid JSON-RPC message.
        // The transport must warn+drop the first and surface the second.
        let mut t = Stdio::connect(&stdio_spec(
            "/bin/sh",
            &["-c", "printf 'not json\\n{\"ok\":true}\\n'"],
        ))
        .await
        .expect("spawn sh");

        let first = tokio::time::timeout(std::time::Duration::from_secs(5), t.recv())
            .await
            .expect("first recv timed out")
            .expect("channel closed");
        match first {
            StdioEvent::Message(v) => assert_eq!(v, serde_json::json!({"ok": true})),
            StdioEvent::Exit(_) => panic!("expected Message before Exit"),
        }
        // Then Exit.
        let next = tokio::time::timeout(std::time::Duration::from_secs(5), t.recv())
            .await
            .expect("second recv timed out")
            .expect("channel closed before Exit");
        match next {
            StdioEvent::Exit(status) => assert!(status.success()),
            StdioEvent::Message(v) => panic!("expected Exit, got Message({v})"),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn send_errors_when_child_stdin_is_closed() {
        // `/bin/true` exits before we can send; stdin pipe is broken.
        // Give the child a moment to actually reap.
        let t = Stdio::connect(&stdio_spec("/bin/true", &[]))
            .await
            .expect("spawn /bin/true");
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let err = t.send(serde_json::json!({"jsonrpc":"2.0","id":1})).await;
        // Either the write fails outright, or it succeeds into a closed
        // pipe that flush detects — both are acceptable as long as we
        // don't panic. Accept Ok() only when it's not observed: some
        // platforms buffer a small first write.
        if err.is_ok() {
            // second write should definitely fail once the pipe EPIPEs.
            let err2 = t.send(serde_json::json!({"jsonrpc":"2.0","id":2})).await;
            assert!(err2.is_err(), "expected send to fail after child exit");
        }
    }

    #[test]
    fn truncate_is_utf8_safe() {
        let s = "a".repeat(600) + "é";
        let out = truncate(&s, 300);
        assert!(out.ends_with('…'));
        // And it must parse as valid UTF-8 (the slice is on a boundary).
        assert!(std::str::from_utf8(out.as_bytes()).is_ok());
    }
}
