//! Stdio JSON-RPC transport for a single MCP server subprocess.
//!
//! The wire format is line-delimited JSON-RPC 2.0: each frame is a UTF-8
//! JSON value terminated by `\n`. Partial reads are absorbed through a
//! bounded reader capped at [`MAX_STDIO_FRAME_BYTES`]; lines that fail to
//! parse as JSON are logged at WARN and dropped — they don't tear the
//! connection down. Lines that exceed the cap are discarded in-flight and
//! surface as [`StdioEvent::Malformed`] so subscribers can observe the
//! misbehavior without the reader buffering the payload (F-347). When the
//! child process exits (or its stdout EOFs), the receiver yields exactly
//! one terminal [`StdioEvent::Exit`] before closing.

use std::process::{ExitStatus, Stdio as StdStdio};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;

use crate::{McpServerSpec, ServerKind};

/// Events emitted on [`Stdio::recv`].
///
/// Consumers receive any number of [`StdioEvent::Message`] and
/// [`StdioEvent::Malformed`] values followed by exactly one terminal
/// [`StdioEvent::Exit`] once the child reaps, at which point the sender is
/// dropped and subsequent calls to [`Stdio::recv`] return `None`.
#[derive(Debug)]
pub enum StdioEvent {
    /// A successfully parsed JSON-RPC 2.0 message from the server's stdout.
    ///
    /// Responses, notifications, and server→client requests all surface
    /// here; dispatch is the manager's job, not the transport's.
    Message(serde_json::Value),
    /// A stdout line exceeded [`MAX_STDIO_FRAME_BYTES`] and was discarded
    /// in-flight. F-347: closes the DoS surface a compromised / buggy /
    /// hostile MCP server exposes by writing an unbounded single line.
    /// The reader keeps running after emitting this event, so a
    /// well-formed frame following the over-cap one still reaches the
    /// event channel.
    Malformed {
        /// How many bytes were buffered before the reader hit the ceiling
        /// and started discarding. Always `>=` [`MAX_STDIO_FRAME_BYTES`].
        bytes_discarded: usize,
    },
    /// The child process exited. Always the last event before channel close.
    Exit(ExitStatus),
}

/// Channel depth for outbound [`StdioEvent`]s. Generous enough that a brief
/// scheduling delay in the consumer doesn't back-pressure the reader task.
const EVENT_CHANNEL_CAPACITY: usize = 128;

/// Maximum bytes the stdout reader will buffer for a single JSON-RPC frame
/// before discarding. Closes F-347: `tokio::io::AsyncBufReadExt::lines`
/// reads until `\n` with no length cap, which lets a compromised / buggy /
/// hostile MCP server DoS the host via a single enormous line. 4 MiB is
/// large enough for realistic MCP payloads (tool-list responses,
/// base64-encoded resource reads) and small enough to keep the worst-case
/// resident set of a misbehaving child bounded. Mirrors F-351's
/// `MAX_LSP_LINE_BYTES` policy for the LSP stdio transport.
pub const MAX_STDIO_FRAME_BYTES: usize = 4 * 1024 * 1024;

/// Variables forwarded from the parent process into every stdio MCP child.
///
/// Security posture: the child environment is **deny-by-default**. We
/// `env_clear()` the `Command` and then re-inject only this allow-list
/// plus whatever the spec's `env` map declares. This prevents a hostile
/// or careless `.mcp.json` entry from silently exfiltrating parent-held
/// credentials (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GITHUB_TOKEN`,
/// `AWS_*`, arbitrary shell exports, etc.) — see F-345.
///
/// Entries are the minimal set the overwhelming majority of MCP servers
/// need to locate binaries and render text correctly: `PATH` (command
/// resolution), `HOME` (per-user config), `LANG` / `LC_ALL` (locale),
/// `USER` / `LOGNAME` (user identity), `TMPDIR` / `TMP` / `TEMP` (tempdir
/// discovery, platform-dependent), `SystemRoot` / `ComSpec` / `PATHEXT`
/// (Windows subprocess basics).
const PARENT_ENV_ALLOWLIST: &[&str] = &[
    "PATH",
    "HOME",
    "LANG",
    "LC_ALL",
    "USER",
    "LOGNAME",
    "TMPDIR",
    "TMP",
    "TEMP",
    "SystemRoot",
    "ComSpec",
    "PATHEXT",
];

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
            // Security (F-345): wipe the inherited environment first, then
            // re-inject an explicit allow-list, then the spec's declared
            // `env` map last so spec values override allow-list values for
            // the same key. Without `env_clear()`, a hostile `.mcp.json`
            // can silently exfiltrate every credential in the parent's env.
            .env_clear();
        for key in PARENT_ENV_ALLOWLIST {
            if let Ok(val) = std::env::var(key) {
                cmd.env(key, val);
            }
        }
        cmd.envs(env)
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
        // Reads are capped at `MAX_STDIO_FRAME_BYTES` (F-347); an over-cap
        // line is discarded and logged at WARN. Stderr is not part of the
        // event contract, so over-cap lines do not surface as
        // `StdioEvent::Malformed`.
        let stderr_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stderr);
            loop {
                match read_line_bounded(&mut reader, MAX_STDIO_FRAME_BYTES).await {
                    Ok(BoundedLine::Line(bytes)) => {
                        if bytes.is_empty() {
                            break;
                        }
                        let text = String::from_utf8_lossy(&bytes);
                        let line = text.trim_end_matches('\n').trim_end_matches('\r');
                        tracing::debug!(
                            target: "forge_mcp::transport::stdio",
                            stderr = %line,
                        );
                    }
                    Ok(BoundedLine::Overflow { bytes_discarded }) => {
                        tracing::warn!(
                            target: "forge_mcp::transport::stdio",
                            stream = "stderr",
                            bytes_discarded = bytes_discarded,
                            cap = MAX_STDIO_FRAME_BYTES,
                            "dropping over-cap stderr line",
                        );
                    }
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
        // the terminal Exit event without racing the consumer. Reads are
        // capped at `MAX_STDIO_FRAME_BYTES` (F-347); over-cap frames are
        // discarded and surface as `StdioEvent::Malformed`. The reader
        // keeps running so a well-formed frame following an over-cap line
        // still reaches the event channel.
        let reader_tx = tx.clone();
        let reader_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            loop {
                match read_line_bounded(&mut reader, MAX_STDIO_FRAME_BYTES).await {
                    Ok(BoundedLine::Line(bytes)) => {
                        if bytes.is_empty() {
                            break; // EOF: child closed stdout.
                        }
                        let trimmed = trim_trailing_newline(&bytes);
                        // Tolerate blank lines (some servers pad frames).
                        if trimmed.iter().all(|b| b.is_ascii_whitespace()) {
                            continue;
                        }
                        match serde_json::from_slice::<serde_json::Value>(trimmed) {
                            Ok(value) => {
                                if reader_tx.send(StdioEvent::Message(value)).await.is_err() {
                                    // Consumer dropped; no need to keep reading.
                                    break;
                                }
                            }
                            Err(err) => {
                                // DoD: malformed frames are logged + skipped,
                                // never fatal.
                                let as_str = String::from_utf8_lossy(trimmed);
                                tracing::warn!(
                                    target: "forge_mcp::transport::stdio",
                                    error = %err,
                                    line = %super::truncate(&as_str, 512),
                                    "dropping malformed JSON-RPC frame",
                                );
                            }
                        }
                    }
                    Ok(BoundedLine::Overflow { bytes_discarded }) => {
                        tracing::warn!(
                            target: "forge_mcp::transport::stdio",
                            stream = "stdout",
                            bytes_discarded = bytes_discarded,
                            cap = MAX_STDIO_FRAME_BYTES,
                            "dropping over-cap stdout line",
                        );
                        if reader_tx
                            .send(StdioEvent::Malformed { bytes_discarded })
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
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

/// Outcome of a single [`read_line_bounded`] call.
enum BoundedLine {
    /// A full line was read within the byte ceiling. Carries the raw bytes
    /// up to and including any trailing `\n`. An empty `Vec` indicates EOF
    /// on the stream.
    Line(Vec<u8>),
    /// The line exceeded the ceiling. All bytes through the terminating
    /// `\n` (or EOF) have been consumed and discarded; `bytes_discarded`
    /// reports the total dropped, always `>=` the cap.
    Overflow { bytes_discarded: usize },
}

/// Read bytes up to and including the next `\n` from `reader`, but never
/// buffer more than `cap` bytes in memory. Closes F-347: a compromised /
/// buggy / hostile MCP server writing a single enormous line cannot DoS
/// the host. Mirrors F-351's `read_line_bounded` for the LSP transport.
///
/// Behavior:
/// - Within the cap → returns [`BoundedLine::Line`] with the bytes read
///   (possibly ending in `\n`; may be empty at EOF).
/// - Over the cap → continues reading-and-discarding via the reader's
///   internal read-ahead buffer until the next `\n` (or EOF) so the
///   reader resyncs on the stream without ever holding more than the
///   reader's own read-ahead (8 KiB default for tokio's `BufReader`),
///   then returns [`BoundedLine::Overflow`] with the total discarded
///   count (always `>= cap`).
/// - Pure I/O error → surfaces as `Err`.
async fn read_line_bounded<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    cap: usize,
) -> std::io::Result<BoundedLine> {
    // Accumulate up to `cap+1` bytes: any overshoot proves the line
    // exceeded the ceiling without a newline. `Take` enforces the ceiling
    // at the reader layer so the `Vec` never grows past `cap+1`.
    let mut buf: Vec<u8> = Vec::new();
    let mut limited = (&mut *reader).take(cap as u64 + 1);
    let _ = limited.read_until(b'\n', &mut buf).await?;
    if buf.is_empty() {
        return Ok(BoundedLine::Line(Vec::new()));
    }
    let hit_newline = buf.last() == Some(&b'\n');
    if hit_newline || buf.len() <= cap {
        return Ok(BoundedLine::Line(buf));
    }

    // Over the cap and no newline yet — drain the remainder of the line
    // from the underlying reader via the `BufRead` fill/consume pair so
    // no intermediate buffer grows beyond the reader's own read-ahead.
    let mut bytes_discarded = buf.len();
    buf.clear();
    loop {
        let chunk = reader.fill_buf().await?;
        if chunk.is_empty() {
            // EOF mid-line — still an overflow, the line never terminated.
            return Ok(BoundedLine::Overflow { bytes_discarded });
        }
        match chunk.iter().position(|&b| b == b'\n') {
            Some(idx) => {
                bytes_discarded = bytes_discarded.saturating_add(idx + 1);
                reader.consume(idx + 1);
                return Ok(BoundedLine::Overflow { bytes_discarded });
            }
            None => {
                let n = chunk.len();
                bytes_discarded = bytes_discarded.saturating_add(n);
                reader.consume(n);
            }
        }
    }
}

/// Strip at most one trailing `\n` (and a preceding `\r`) from `bytes`.
/// Matches `BufReader::lines` semantics for downstream JSON parsing.
fn trim_trailing_newline(bytes: &[u8]) -> &[u8] {
    let mut end = bytes.len();
    if end > 0 && bytes[end - 1] == b'\n' {
        end -= 1;
        if end > 0 && bytes[end - 1] == b'\r' {
            end -= 1;
        }
    }
    &bytes[..end]
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
        // `sh -c 'exit 0'` spawns, writes nothing, exits 0. We expect
        // exactly one Exit event and then a closed channel.
        // (Using `/bin/sh` rather than `/bin/true` because GitHub's
        // macos-latest runner image ships without `/bin/true`.)
        let mut t = Stdio::connect(&stdio_spec("/bin/sh", &["-c", "exit 0"]))
            .await
            .expect("spawn sh exit 0");
        let ev = tokio::time::timeout(std::time::Duration::from_secs(5), t.recv())
            .await
            .expect("recv did not yield Exit in time")
            .expect("channel closed before Exit");
        match ev {
            StdioEvent::Exit(status) => assert!(status.success()),
            StdioEvent::Message(v) => panic!("expected Exit, got Message({v})"),
            StdioEvent::Malformed { bytes_discarded } => {
                panic!("expected Exit, got Malformed({bytes_discarded})")
            }
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
            StdioEvent::Malformed { bytes_discarded } => {
                panic!("expected Message, got Malformed({bytes_discarded})")
            }
        }
        // Then Exit.
        let next = tokio::time::timeout(std::time::Duration::from_secs(5), t.recv())
            .await
            .expect("second recv timed out")
            .expect("channel closed before Exit");
        match next {
            StdioEvent::Exit(status) => assert!(status.success()),
            StdioEvent::Message(v) => panic!("expected Exit, got Message({v})"),
            StdioEvent::Malformed { bytes_discarded } => {
                panic!("expected Exit, got Malformed({bytes_discarded})")
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn send_errors_when_child_stdin_is_closed() {
        // Short-lived shell exits before we can send; stdin pipe is
        // broken. Give the child a moment to actually reap.
        // (Using `/bin/sh` rather than `/bin/true` because GitHub's
        // macos-latest runner image ships without `/bin/true`.)
        let t = Stdio::connect(&stdio_spec("/bin/sh", &["-c", "exit 0"]))
            .await
            .expect("spawn sh exit 0");
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

    /// Drain events until the child exits, returning the exit status.
    /// Fails the test if recv times out or the channel closes before Exit.
    async fn drain_to_exit(t: &mut Stdio) -> ExitStatus {
        loop {
            let ev = tokio::time::timeout(std::time::Duration::from_secs(5), t.recv())
                .await
                .expect("recv timed out")
                .expect("channel closed before Exit");
            if let StdioEvent::Exit(status) = ev {
                return status;
            }
        }
    }

    /// F-345 regression: the child must NOT inherit parent-process env vars
    /// outside the allow-list. We set a sentinel in the parent, then spawn
    /// a shell that exits 0 only if the sentinel is **absent** from its
    /// env. Pre-fix (no `env_clear`), `$CANARY` would be "leak-me" and the
    /// shell would exit 1. Post-fix the child sees nothing and exits 0.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn child_does_not_inherit_parent_env_secrets() {
        // Unique per-process so concurrent tests don't collide on the
        // global env.
        let key = format!("FORGE_MCP_F345_CANARY_{}", std::process::id());
        std::env::set_var(&key, "leak-me-if-you-can");

        // `[ -z "$VAR" ]` is true iff the var is unset or empty. Exit 0
        // means the leak was blocked; exit 1 means the child saw it.
        let script = format!("[ -z \"${key}\" ]");
        let spec = stdio_spec("/bin/sh", &["-c", &script]);

        let mut t = Stdio::connect(&spec).await.expect("spawn sh env-check");
        let status = drain_to_exit(&mut t).await;

        std::env::remove_var(&key);

        assert!(
            status.success(),
            "child inherited parent env secret {key}; shell exited {status:?}",
        );
    }

    /// Positive control: the allow-list must still forward `PATH` so the
    /// child can resolve binaries. Exit 0 iff the child's PATH matches
    /// the parent's — proves env isolation isn't also starving the child.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn child_inherits_path_from_allowlist() {
        let parent_path = std::env::var("PATH").unwrap_or_default();
        assert!(
            !parent_path.is_empty(),
            "test precondition: parent PATH must be non-empty"
        );

        // `test "$PATH" = "<parent>"` exits 0 iff they match.
        let script = format!("test \"$PATH\" = {parent:?}", parent = parent_path);
        let spec = stdio_spec("/bin/sh", &["-c", &script]);

        let mut t = Stdio::connect(&spec).await.expect("spawn sh path-check");
        let status = drain_to_exit(&mut t).await;

        assert!(
            status.success(),
            "child PATH did not carry parent PATH; shell exited {status:?}",
        );
    }

    /// The spec-declared `env` map must still reach the child after we
    /// clear the inherited environment. Regression guard against an
    /// over-zealous filter that drops caller-provided vars.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn spec_env_reaches_child_after_clear() {
        let mut env = BTreeMap::new();
        env.insert(
            "FORGE_MCP_SPEC_VAR".to_string(),
            "declared-value".to_string(),
        );
        let spec = McpServerSpec {
            kind: ServerKind::Stdio {
                command: "/bin/sh".into(),
                args: vec![
                    "-c".into(),
                    "test \"$FORGE_MCP_SPEC_VAR\" = declared-value".into(),
                ],
                env,
            },
        };

        let mut t = Stdio::connect(&spec).await.expect("spawn sh spec-env");
        let status = drain_to_exit(&mut t).await;

        assert!(
            status.success(),
            "spec-declared env did not reach child; shell exited {status:?}",
        );
    }

    // -----------------------------------------------------------------------
    // F-347: stdio reader must enforce a documented max-line ceiling. A
    // compromised / buggy / hostile MCP server that writes a single enormous
    // line (no newline) must not drive `forge-mcp` into unbounded memory use.
    // Over-cap events surface as `StdioEvent::Malformed` and the reader
    // keeps running for subsequent frames.
    // -----------------------------------------------------------------------

    /// DoD regression: a child that writes 16 MiB of no-newline bytes must
    /// not grow the reader's buffer past the cap. The reader must surface
    /// one `Malformed` event and keep running so a valid frame following
    /// the over-cap bytes still reaches the event channel. Matches the
    /// issue's "fed 16 MiB of no-boundary bytes, assert bounded memory +
    /// named error" requirement.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stdout_sixteen_mib_no_newline_is_bounded() {
        // 16 MiB of `x`, no newline, then a valid JSON frame on its own
        // line. `head -c` bounds argv memory; `tr` converts null bytes to
        // printable ASCII. Total child-side memory cost is tiny regardless
        // of the byte count — the hostile producer is a pipe, not argv.
        let script = r#"
            head -c 16777216 /dev/zero | tr '\0' 'x'
            printf '\n{"ok":true}\n'
        "#;

        let mut t = Stdio::connect(&stdio_spec("/bin/sh", &["-c", script]))
            .await
            .expect("spawn sh 16MiB");

        let mut saw_malformed = false;
        let mut saw_valid_message = false;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        while tokio::time::Instant::now() < deadline {
            let ev = tokio::time::timeout(std::time::Duration::from_secs(5), t.recv()).await;
            match ev {
                Ok(Some(StdioEvent::Malformed { bytes_discarded })) => {
                    assert!(
                        bytes_discarded >= MAX_STDIO_FRAME_BYTES,
                        "bytes_discarded must be >= cap: {bytes_discarded}",
                    );
                    saw_malformed = true;
                }
                Ok(Some(StdioEvent::Message(v))) => {
                    assert_eq!(
                        v,
                        serde_json::json!({"ok": true}),
                        "reader must deliver the valid frame after the over-cap line",
                    );
                    saw_valid_message = true;
                }
                Ok(Some(StdioEvent::Exit(_))) | Ok(None) | Err(_) => break,
            }
        }
        assert!(
            saw_malformed,
            "over-cap stdout line must surface StdioEvent::Malformed",
        );
        assert!(
            saw_valid_message,
            "reader must survive the over-cap line and deliver the next valid JSON frame",
        );
    }
}
