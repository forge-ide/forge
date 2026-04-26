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

use std::process::ExitStatus;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use forge_core::process::{ManagedStdioChild, ManagedStdioConfig, StdioChildEvent};
use tokio::io::AsyncWriteExt;
use tokio::process::ChildStdin;
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
/// scheduling delay in the consumer doesn't back-pressure the relay task.
const EVENT_CHANNEL_CAPACITY: usize = 128;

/// Maximum bytes the stdout reader will buffer for a single JSON-RPC frame
/// before discarding. Closes F-347: `tokio::io::AsyncBufReadExt::lines`
/// reads until `\n` with no length cap, which lets a compromised / buggy /
/// hostile MCP server DoS the host via a single enormous line. 4 MiB is
/// large enough for realistic MCP payloads (tool-list responses,
/// base64-encoded resource reads) and small enough to keep the worst-case
/// resident set of a misbehaving child bounded. Mirrors F-351's
/// `MAX_LSP_LINE_BYTES` policy for the LSP stdio transport. The cap is
/// enforced inside the [`ManagedStdioChild`] reader; this constant is the
/// MCP-shaped contract surface that callers and tests assert against.
pub const MAX_STDIO_FRAME_BYTES: usize = 4 * 1024 * 1024;

/// An active stdio JSON-RPC connection to one MCP server subprocess.
///
/// Construct with [`Stdio::connect`]. The handle owns the child's stdin
/// (for [`Stdio::send`]) and a receiver of [`StdioEvent`]s relayed from
/// the underlying [`ManagedStdioChild`]. The shared primitive owns the
/// child process, drains stderr at DEBUG, parses stdout into JSON
/// frames, and reaps the child after stdout EOF.
pub struct Stdio {
    /// Protected so concurrent senders serialise their frames; JSON-RPC
    /// frames must be atomic relative to `\n`.
    stdin: Arc<Mutex<ChildStdin>>,
    rx: mpsc::Receiver<StdioEvent>,
    /// Translator task that maps `StdioChildEvent` → `StdioEvent`. Held
    /// purely to tie its lifetime to this struct; it ends naturally when
    /// the underlying primitive emits `Exit`. We do NOT abort it on drop
    /// for the same reason `ManagedStdioChild` doesn't abort its reader:
    /// it owns the only handle that observes the terminal `Exit` event,
    /// and aborting mid-await would lose that observation.
    _relay: JoinHandle<()>,
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
    ///
    /// Spawn / env-scrub / stderr-drain / reader / reap all live in
    /// [`forge_core::process::ManagedStdioChild`]; this method is the
    /// MCP-shaped facade that adds line-delimited JSON-RPC framing on
    /// top of the shared primitive.
    pub async fn connect(spec: &McpServerSpec) -> Result<Self> {
        let (command, args, env) = match &spec.kind {
            ServerKind::Stdio { command, args, env } => (command, args, env),
            ServerKind::Http { .. } => {
                return Err(anyhow!(
                    "stdio transport cannot connect to an http MCP server"
                ));
            }
        };

        let cfg = ManagedStdioConfig {
            program: command.into(),
            args: args.iter().map(Into::into).collect(),
            extra_env: env.clone(),
            // The MCP transport logs through the primitive's own
            // tracing target (`forge_core::process`); per-server
            // disambiguation is left to the manager's tracing context.
            server_id: None,
            // F-347: bound the per-frame ceiling at 4 MiB so a
            // compromised / buggy / hostile MCP server cannot DoS the
            // host with an unterminated line. Surfaces over-cap stdout
            // as `StdioChildEvent::MalformedStdout`, which the relay
            // below maps to `StdioEvent::Malformed`.
            max_frame_bytes: Some(MAX_STDIO_FRAME_BYTES),
        };
        let mut managed = ManagedStdioChild::spawn(cfg)
            .with_context(|| format!("spawning MCP server {command:?}"))?;

        // Lift the primitive's `ChildStdin` into the `Arc<Mutex<…>>`
        // shape MCP's `send` API requires. The remainder of `managed`
        // (events receiver + reader/stderr task handles) is moved into
        // the relay task below so the underlying primitive stays alive
        // until the terminal `Exit` event is observed.
        let stdin = managed
            .take_stdin()
            .ok_or_else(|| anyhow!("managed stdio child had no stdin pipe"))?;

        // Translate the primitive's `StdioChildEvent` enum into this
        // crate's `StdioEvent` so the public API stays unchanged. The
        // primitive's reader does the heavy lifting (parse, drain,
        // reap); this task is just a thin re-channel. The relay task
        // owns `managed` so the underlying `ManagedStdioChild` stays
        // alive (and its reader continues to wait on the child) until
        // the terminal `Exit` event has been observed and forwarded.
        let (tx, rx) = mpsc::channel::<StdioEvent>(EVENT_CHANNEL_CAPACITY);
        let relay = tokio::spawn(async move {
            while let Some(ev) = managed.events.recv().await {
                let mapped = match ev {
                    StdioChildEvent::Message(v) => StdioEvent::Message(v),
                    StdioChildEvent::Exit(s) => StdioEvent::Exit(s),
                    // F-347: stdout over-cap surfaces to MCP consumers; the
                    // primitive already logged the discard.
                    StdioChildEvent::MalformedStdout { bytes_discarded } => {
                        StdioEvent::Malformed { bytes_discarded }
                    }
                    // Stderr over-cap is not part of the MCP event contract;
                    // the primitive already logged it. Skip the relay so we
                    // don't fabricate a `Malformed` for a non-frame stream.
                    StdioChildEvent::MalformedStderr { .. } => continue,
                };
                if tx.send(mapped).await.is_err() {
                    break;
                }
            }
            // `managed` drops here: the primitive's reader has already
            // reaped the child by this point (it sent us the Exit
            // event), so this drop is just bookkeeping.
            drop(managed);
        });

        Ok(Self {
            stdin: Arc::new(Mutex::new(stdin)),
            rx,
            _relay: relay,
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
