//! Subprocess scaffolding shared by `forge-mcp` and `forge-lsp`.
//!
//! Both crates supervise a long-running child that speaks newline-delimited
//! JSON on stdio. They previously duplicated the spawn / env-scrub / stderr
//! drain / stdout reader / reap dance with only incidental policy
//! differences. [`ManagedStdioChild::spawn`] is the single shared
//! primitive: it spawns the child with a deny-by-default environment
//! (allow-listed parents + caller-declared extras), drains stderr at
//! `DEBUG`, parses stdout into [`StdioChildEvent::Message`] frames, and
//! emits a terminal [`StdioChildEvent::Exit`] once the child reaps.
//!
//! Per-crate concerns stay with each caller:
//! - `forge-mcp` wraps the primitive in a `Stdio` handle that owns
//!   `Arc<Mutex<ChildStdin>>` and exposes `send` / `recv`.
//! - `forge-lsp` wraps it in a supervisor that swaps a `ChildStdin` in
//!   and out of a `StdioTransport` across restart-with-backoff attempts.
//!
//! The wire framing (line-delimited JSON) is identical between the two
//! crates today; the primitive enforces it so neither crate can drift.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{ExitStatus, Stdio as StdStdio};

use anyhow::{anyhow, Context, Result};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::mpsc;
use tokio::task::{AbortHandle, JoinHandle};

/// Variables forwarded from the parent process into every spawned child.
///
/// Security posture: the child environment is **deny-by-default**.
/// [`ManagedStdioChild::spawn`] calls `env_clear()` on the `Command` and
/// then re-injects only this allow-list plus the caller's `extra_env`. This
/// prevents a hostile or careless server config from silently exfiltrating
/// parent-held credentials (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`,
/// `GITHUB_TOKEN`, `AWS_*`, arbitrary shell exports) — F-345 / F-353.
///
/// Entries are the minimal set the overwhelming majority of stdio servers
/// need to locate binaries and render text correctly: `PATH` (command
/// resolution), `HOME` (per-user config), `LANG` / `LC_ALL` (locale),
/// `USER` / `LOGNAME` (user identity), `TMPDIR` / `TMP` / `TEMP` (tempdir
/// discovery, platform-dependent), `SystemRoot` / `ComSpec` / `PATHEXT`
/// (Windows subprocess basics).
pub const PARENT_ENV_ALLOWLIST: &[&str] = &[
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

/// Channel depth for outbound [`StdioChildEvent`]s. Generous enough that a
/// brief scheduling delay in the consumer doesn't back-pressure the reader.
const EVENT_CHANNEL_CAPACITY: usize = 128;

/// Maximum bytes preserved when logging a malformed stdout frame, so a
/// runaway server can't flood the log ring.
const MAX_LOGGED_FRAME_BYTES: usize = 512;

/// Cap a log field at `max` bytes so a runaway frame can't flood the log
/// ring. Input is assumed UTF-8; slicing is done on a char boundary via
/// `char_indices` to avoid panicking on multi-byte glyphs.
pub fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    for (i, _) in s.char_indices() {
        if i > max {
            break;
        }
        end = i;
    }
    format!("{}…", &s[..end])
}

/// Events emitted by [`ManagedStdioChild`] on its `events` receiver.
///
/// Consumers receive any number of [`StdioChildEvent::Message`],
/// [`StdioChildEvent::MalformedStdout`], and [`StdioChildEvent::MalformedStderr`]
/// values followed by exactly one terminal [`StdioChildEvent::Exit`] once
/// the child reaps, at which point the sender is dropped and subsequent
/// `recv()` calls return `None`.
#[derive(Debug)]
pub enum StdioChildEvent {
    /// A successfully parsed JSON value from the child's stdout.
    Message(serde_json::Value),
    /// A stdout line exceeded `cfg.max_frame_bytes` and was discarded
    /// in-flight. Closes F-347 (MCP) / F-351 (LSP): a compromised / buggy /
    /// hostile child writing a single enormous line cannot DoS the host.
    /// The reader keeps running so a well-formed frame following the
    /// over-cap line still reaches the channel.
    MalformedStdout {
        /// Bytes buffered before the reader hit the ceiling and started
        /// discarding. Always `>=` the configured cap.
        bytes_discarded: usize,
    },
    /// A stderr line exceeded `cfg.max_frame_bytes` and was discarded
    /// in-flight. Stderr is free-form logs, but the cap still applies so
    /// the drain task can never buffer past the ceiling. Distinct from
    /// `MalformedStdout` so consumers can route stdout-frame DoS and
    /// stderr-log-flood differently.
    MalformedStderr {
        /// Bytes buffered before the reader hit the ceiling and started
        /// discarding. Always `>=` the configured cap.
        bytes_discarded: usize,
    },
    /// The child process exited. Always the last event before channel close.
    Exit(ExitStatus),
}

/// Caller-provided spawn parameters.
///
/// `extra_env` entries override allow-list values for the same key (this
/// matches both crates' prior behaviour: spec-declared env wins).
#[derive(Debug, Clone)]
pub struct ManagedStdioConfig {
    /// Program to execute. Resolved via `PATH` if not absolute.
    pub program: OsString,
    /// Argv passed to the child.
    pub args: Vec<OsString>,
    /// Caller-declared environment overlay. Applied **after** the
    /// allow-list so spec-declared values override parent values for the
    /// same key.
    pub extra_env: BTreeMap<String, String>,
    /// Optional structured `server_id` field included on every log line
    /// emitted by the drain / reader tasks. `forge-lsp` uses this for
    /// per-server disambiguation (F-386). `forge-mcp` leaves it unset
    /// because it logs through its own per-server tracing context.
    pub server_id: Option<String>,
    /// Optional per-line byte ceiling enforced on both stdout and stderr.
    /// `None` means the reader uses an unbounded `read_until` (matches
    /// pre-F-347/F-351 behaviour). `Some(cap)` switches both readers to a
    /// bounded read that discards over-cap lines and surfaces them as
    /// [`StdioChildEvent::MalformedStdout`] / [`StdioChildEvent::MalformedStderr`].
    /// MCP sets this to its 4 MiB `MAX_STDIO_FRAME_BYTES` (F-347); LSP sets
    /// it to its 4 MiB `MAX_LSP_LINE_BYTES` (F-351).
    pub max_frame_bytes: Option<usize>,
}

/// One spawned, supervised stdio child.
///
/// Construct with [`ManagedStdioChild::spawn`]. The handle owns the
/// child's `stdin` (caller-driven sender) and an `events` receiver that
/// surfaces parsed JSON frames and the terminal exit status. The reader
/// task internally `wait()`s on the child after stdout EOF, so the caller
/// never has to call `kill` or `wait` directly: dropping the handle
/// triggers `Command::kill_on_drop(true)` and the reader reaps.
///
/// The reader join handle is held purely to tie the task's lifetime to
/// this struct. We deliberately do **not** `.abort()` it on drop: the
/// reader is the only task that `wait()`s on the child, and aborting
/// mid-wait leaves the child un-reaped — later spawns in the same tokio
/// runtime then observe zombie handles and stall. Relying on
/// `kill_on_drop(true)` plus the reader's own end-of-stream `wait()` is
/// both necessary and sufficient.
pub struct ManagedStdioChild {
    /// Child's stdin pipe. Wrapped in `Option` so callers can `.take()`
    /// it without losing the rest of the struct (which keeps the
    /// reader/stderr tasks alive). Each crate wraps the taken pipe in
    /// whatever sharing/swapping policy fits its lifecycle: `forge-mcp`
    /// holds it as `Arc<Mutex<ChildStdin>>`; `forge-lsp` swaps it in and
    /// out of a `StdioTransport` across restart attempts.
    stdin: Option<ChildStdin>,
    /// Inbound JSON frames followed by a single terminal `Exit`.
    pub events: mpsc::Receiver<StdioChildEvent>,
    /// Reader task that owns the child + stdout. See struct docs for why
    /// this is held without aborting on drop.
    _reader: JoinHandle<()>,
    /// Stderr drain task. Aborted on drop because it holds no child
    /// handle — only a log-forwarding loop over the closed-on-drop pipe.
    /// The reader task `await`s the same task by `JoinHandle` so a
    /// pending `MalformedStderr` event lands before the terminal `Exit`
    /// (matches F-351's "stderr drain joined before Exited" ordering).
    stderr: AbortHandle,
}

impl Drop for ManagedStdioChild {
    fn drop(&mut self) {
        // Reader is left alone so it finishes `child.wait()`; stderr is
        // aborted so the runtime reclaims its blocking slot promptly.
        self.stderr.abort();
    }
}

impl std::fmt::Debug for ManagedStdioChild {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagedStdioChild").finish_non_exhaustive()
    }
}

impl ManagedStdioChild {
    /// Spawn `cfg.program` with the configured argv + env policy and
    /// connect a JSON event channel to its stdio.
    ///
    /// Errors if the subprocess cannot be spawned (missing binary, PATH
    /// issue, etc.) or if any of the three stdio pipes are missing.
    pub fn spawn(cfg: ManagedStdioConfig) -> Result<Self> {
        let mut cmd = Command::new(&cfg.program);
        cmd.args(&cfg.args)
            // Security (F-345 / F-353): wipe the inherited environment
            // first, then re-inject only the explicit allow-list, then
            // the caller's `extra_env` last so spec values override
            // allow-list values for the same key.
            .env_clear();
        for key in PARENT_ENV_ALLOWLIST {
            if let Ok(val) = std::env::var(key) {
                cmd.env(key, val);
            }
        }
        cmd.envs(&cfg.extra_env)
            .stdin(StdStdio::piped())
            .stdout(StdStdio::piped())
            .stderr(StdStdio::piped())
            // Ensure the child is killed if the handle is dropped before
            // an explicit shutdown. These children are long-running;
            // without this they'd leak past the parent on panic / early
            // return.
            .kill_on_drop(true);

        let program_for_err = PathBuf::from(&cfg.program);
        let mut child = cmd
            .spawn()
            .with_context(|| format!("spawning stdio child {:?}", program_for_err.display()))?;

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

        let (tx, rx) = mpsc::channel::<StdioChildEvent>(EVENT_CHANNEL_CAPACITY);

        let server_id_for_stderr = cfg.server_id.clone();
        let server_id_for_reader = cfg.server_id.clone();
        let max_frame_bytes = cfg.max_frame_bytes;

        // Stderr drain: prevents the child's stderr pipe from filling up
        // and blocking its writes. Log at DEBUG — stderr is free-form.
        // The optional `server_id` field disambiguates concurrent
        // children in the log ring (F-386); when absent, the field is
        // omitted entirely so the empty-id case stays terse. Reads
        // honour `cfg.max_frame_bytes` (F-351): an over-cap line is
        // discarded and surfaces as `StdioChildEvent::MalformedStderr`
        // so callers can route the event (e.g. LSP exposes it; MCP
        // ignores it).
        let stderr_tx = tx.clone();
        let stderr_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stderr);
            loop {
                match next_line(&mut reader, max_frame_bytes).await {
                    Ok(LineOutcome::Line(bytes)) => {
                        if bytes.is_empty() {
                            break;
                        }
                        let text = String::from_utf8_lossy(&bytes);
                        let line = text.trim_end_matches('\n').trim_end_matches('\r');
                        match server_id_for_stderr.as_deref() {
                            Some(id) => tracing::debug!(
                                target: "forge_core::process",
                                server_id = %id,
                                stderr = %line,
                            ),
                            None => tracing::debug!(
                                target: "forge_core::process",
                                stderr = %line,
                            ),
                        }
                    }
                    Ok(LineOutcome::Overflow { bytes_discarded }) => {
                        match server_id_for_stderr.as_deref() {
                            Some(id) => tracing::warn!(
                                target: "forge_core::process",
                                server_id = %id,
                                stream = "stderr",
                                bytes_discarded = bytes_discarded,
                                "dropping over-cap stderr line",
                            ),
                            None => tracing::warn!(
                                target: "forge_core::process",
                                stream = "stderr",
                                bytes_discarded = bytes_discarded,
                                "dropping over-cap stderr line",
                            ),
                        }
                        if stderr_tx
                            .send(StdioChildEvent::MalformedStderr { bytes_discarded })
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(err) => {
                        match server_id_for_stderr.as_deref() {
                            Some(id) => tracing::debug!(
                                target: "forge_core::process",
                                server_id = %id,
                                error = %err,
                                "stderr read error; draining aborted",
                            ),
                            None => tracing::debug!(
                                target: "forge_core::process",
                                error = %err,
                                "stderr read error; draining aborted",
                            ),
                        }
                        break;
                    }
                }
            }
        });

        // Reader: owns stdout + child, so it can wait() after EOF and
        // emit the terminal Exit event without racing the consumer. Uses
        // `next_line(&mut reader, max_frame_bytes)`: when `max_frame_bytes`
        // is `Some`, the reader is bounded and over-cap lines surface as
        // `StdioChildEvent::MalformedStdout` (F-347 / F-351); otherwise the
        // reader uses an unbounded `read_until` with a task-scoped reusable
        // `Vec<u8>` (F-574). Behaviour matches both crates' prior
        // line-by-line readers (trailing CR/LF stripped, blank lines
        // skipped, malformed JSON warn-and-drop).
        //
        // `stderr_task` is moved into the reader so it can be awaited
        // before the terminal `Exit` event fires — that way any pending
        // `MalformedStderr` lands in the channel ahead of `Exit` and the
        // last sender drops in a deterministic order (F-351 ordering).
        // Drop's `abort()` still fires on early teardown via the held
        // `AbortHandle`.
        let stderr_abort = stderr_task.abort_handle();
        let reader_tx = tx.clone();
        // Last sender is the reader-owned `tx`. Drop the original after
        // the clones are taken so the channel closes once the reader and
        // stderr task release their senders.
        drop(tx);
        let reader_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            loop {
                match next_line(&mut reader, max_frame_bytes).await {
                    Ok(LineOutcome::Line(bytes)) => {
                        if bytes.is_empty() {
                            break; // EOF: child closed stdout.
                        }
                        // Strip the trailing `\n` (and any preceding `\r`)
                        // so JSON parsing sees a clean frame.
                        let mut end = bytes.len();
                        if end > 0 && bytes[end - 1] == b'\n' {
                            end -= 1;
                        }
                        if end > 0 && bytes[end - 1] == b'\r' {
                            end -= 1;
                        }
                        let frame = &bytes[..end];
                        // Tolerate blank / whitespace-only lines (some
                        // servers pad frames).
                        if frame.iter().all(|b| b.is_ascii_whitespace()) {
                            continue;
                        }
                        match serde_json::from_slice::<serde_json::Value>(frame) {
                            Ok(value) => {
                                if reader_tx
                                    .send(StdioChildEvent::Message(value))
                                    .await
                                    .is_err()
                                {
                                    // Consumer dropped; no need to keep reading.
                                    break;
                                }
                            }
                            Err(err) => {
                                let line = String::from_utf8_lossy(frame);
                                let truncated = truncate(line.as_ref(), MAX_LOGGED_FRAME_BYTES);
                                match server_id_for_reader.as_deref() {
                                    Some(id) => tracing::warn!(
                                        target: "forge_core::process",
                                        server_id = %id,
                                        error = %err,
                                        line = %truncated,
                                        "dropping malformed stdout frame",
                                    ),
                                    None => tracing::warn!(
                                        target: "forge_core::process",
                                        error = %err,
                                        line = %truncated,
                                        "dropping malformed stdout frame",
                                    ),
                                }
                            }
                        }
                    }
                    Ok(LineOutcome::Overflow { bytes_discarded }) => {
                        match server_id_for_reader.as_deref() {
                            Some(id) => tracing::warn!(
                                target: "forge_core::process",
                                server_id = %id,
                                stream = "stdout",
                                bytes_discarded = bytes_discarded,
                                "dropping over-cap stdout line",
                            ),
                            None => tracing::warn!(
                                target: "forge_core::process",
                                stream = "stdout",
                                bytes_discarded = bytes_discarded,
                                "dropping over-cap stdout line",
                            ),
                        }
                        if reader_tx
                            .send(StdioChildEvent::MalformedStdout { bytes_discarded })
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(err) => {
                        match server_id_for_reader.as_deref() {
                            Some(id) => tracing::warn!(
                                target: "forge_core::process",
                                server_id = %id,
                                error = %err,
                                "stdout read error; surfacing as exit",
                            ),
                            None => tracing::warn!(
                                target: "forge_core::process",
                                error = %err,
                                "stdout read error; surfacing as exit",
                            ),
                        }
                        break;
                    }
                }
            }

            // Reap the child and surface the exit status. `wait()` is
            // safe to call after we've dropped stdout — the pipe is
            // closed.
            let status = match child.wait().await {
                Ok(s) => s,
                Err(err) => {
                    match server_id_for_reader.as_deref() {
                        Some(id) => tracing::warn!(
                            target: "forge_core::process",
                            server_id = %id,
                            error = %err,
                            "wait() failed; forcing kill",
                        ),
                        None => tracing::warn!(
                            target: "forge_core::process",
                            error = %err,
                            "wait() failed; forcing kill",
                        ),
                    }
                    let _ = child.kill().await;
                    child
                        .wait()
                        .await
                        .unwrap_or_else(|_| std::process::ExitStatus::default())
                }
            };

            // Drain the stderr task before announcing Exit so any
            // pending `MalformedStderr` (or final stderr line) lands in
            // the channel ahead of the terminal event. Without this a
            // consumer that polls until `Exit` and stops would miss the
            // last stderr-sourced events even though they happened
            // before the child reaped.
            let _ = stderr_task.await;

            // `send` may fail if the consumer dropped — that's fine.
            let _ = reader_tx.send(StdioChildEvent::Exit(status)).await;
        });

        Ok(Self {
            stdin: Some(stdin),
            events: rx,
            _reader: reader_task,
            stderr: stderr_abort,
        })
    }

    /// Take ownership of the child's stdin pipe. The first call returns
    /// `Some(ChildStdin)`; subsequent calls return `None`. Callers wrap
    /// the returned handle in whatever sharing / swapping policy their
    /// lifecycle requires.
    pub fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.stdin.take()
    }
}

/// Outcome of a single [`next_line`] call.
enum LineOutcome {
    /// A full line within the (possibly absent) ceiling. Carries the raw
    /// bytes up to and including any trailing `\n`. An empty `Vec`
    /// indicates EOF on the stream.
    Line(Vec<u8>),
    /// The line exceeded the ceiling. All bytes through the terminating
    /// `\n` (or EOF) have been consumed and discarded; `bytes_discarded`
    /// reports the total dropped, always `>=` the cap.
    Overflow { bytes_discarded: usize },
}

/// Read the next `\n`-terminated line from `reader`. When `cap` is
/// `Some`, the read is bounded: a line longer than `cap` is drained
/// (without buffering past the reader's own read-ahead) and surfaces as
/// [`LineOutcome::Overflow`]. With `None` the read is unbounded — matches
/// the pre-F-347/F-351 `read_until` behaviour.
///
/// Closes F-347 (MCP) / F-351 (LSP): a compromised / buggy / hostile
/// child writing a single enormous line cannot DoS the host.
async fn next_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    cap: Option<usize>,
) -> std::io::Result<LineOutcome> {
    let mut buf: Vec<u8> = Vec::new();
    match cap {
        None => {
            let _ = reader.read_until(b'\n', &mut buf).await?;
            Ok(LineOutcome::Line(buf))
        }
        Some(cap) => {
            // Accumulate up to `cap+1` bytes: any overshoot proves the
            // line exceeded the ceiling without a newline. `Take` enforces
            // the ceiling at the reader layer so the `Vec` never grows
            // past `cap+1`.
            let mut limited = (&mut *reader).take(cap as u64 + 1);
            let _ = limited.read_until(b'\n', &mut buf).await?;
            if buf.is_empty() {
                return Ok(LineOutcome::Line(Vec::new()));
            }
            let hit_newline = buf.last() == Some(&b'\n');
            if hit_newline || buf.len() <= cap {
                return Ok(LineOutcome::Line(buf));
            }

            // Over the cap and no newline yet — drain the remainder of
            // the line via the reader's `BufRead` fill/consume pair so no
            // intermediate buffer grows beyond the reader's own
            // read-ahead (8 KiB default for tokio's `BufReader`).
            let mut bytes_discarded = buf.len();
            buf.clear();
            loop {
                let chunk = reader.fill_buf().await?;
                if chunk.is_empty() {
                    return Ok(LineOutcome::Overflow { bytes_discarded });
                }
                match chunk.iter().position(|&b| b == b'\n') {
                    Some(idx) => {
                        bytes_discarded = bytes_discarded.saturating_add(idx + 1);
                        reader.consume(idx + 1);
                        return Ok(LineOutcome::Overflow { bytes_discarded });
                    }
                    None => {
                        let n = chunk.len();
                        bytes_discarded = bytes_discarded.saturating_add(n);
                        reader.consume(n);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(program: &str, args: &[&str]) -> ManagedStdioConfig {
        ManagedStdioConfig {
            program: program.into(),
            args: args.iter().map(|s| OsString::from(*s)).collect(),
            extra_env: BTreeMap::new(),
            server_id: None,
            max_frame_bytes: None,
        }
    }

    #[test]
    fn truncate_is_utf8_safe() {
        let s = "a".repeat(600) + "é";
        let out = truncate(&s, 300);
        assert!(out.ends_with('…'));
        assert!(std::str::from_utf8(out.as_bytes()).is_ok());
    }

    #[test]
    fn truncate_is_noop_below_cap() {
        assert_eq!(truncate("short", 64), "short");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn surfaces_exit_event_when_child_exits_immediately() {
        let mut child =
            ManagedStdioChild::spawn(cfg("/bin/sh", &["-c", "exit 0"])).expect("spawn sh exit 0");
        let ev = tokio::time::timeout(std::time::Duration::from_secs(5), child.events.recv())
            .await
            .expect("recv did not yield Exit in time")
            .expect("channel closed before Exit");
        match ev {
            StdioChildEvent::Exit(status) => assert!(status.success()),
            other => panic!("expected Exit, got {other:?}"),
        }
        assert!(
            child.events.recv().await.is_none(),
            "channel must close after Exit"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn drops_malformed_lines_without_closing_stream() {
        let mut child = ManagedStdioChild::spawn(cfg(
            "/bin/sh",
            &["-c", "printf 'not json\\n{\"ok\":true}\\n'"],
        ))
        .expect("spawn sh");

        let first = tokio::time::timeout(std::time::Duration::from_secs(5), child.events.recv())
            .await
            .expect("first recv timed out")
            .expect("channel closed");
        match first {
            StdioChildEvent::Message(v) => assert_eq!(v, serde_json::json!({"ok": true})),
            other => panic!("expected Message before Exit, got {other:?}"),
        }
        let next = tokio::time::timeout(std::time::Duration::from_secs(5), child.events.recv())
            .await
            .expect("second recv timed out")
            .expect("channel closed before Exit");
        match next {
            StdioChildEvent::Exit(status) => assert!(status.success()),
            other => panic!("expected Exit, got {other:?}"),
        }
    }

    /// F-345 regression: the child must NOT inherit parent-process env vars
    /// outside the allow-list.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn child_does_not_inherit_parent_env_secrets() {
        let key = format!("FORGE_CORE_PROCESS_CANARY_{}", std::process::id());
        std::env::set_var(&key, "leak-me-if-you-can");

        let script = format!("[ -z \"${key}\" ]");
        let mut child =
            ManagedStdioChild::spawn(cfg("/bin/sh", &["-c", &script])).expect("spawn sh env-check");

        let status = loop {
            let ev = tokio::time::timeout(std::time::Duration::from_secs(5), child.events.recv())
                .await
                .expect("recv timed out")
                .expect("channel closed before Exit");
            if let StdioChildEvent::Exit(s) = ev {
                break s;
            }
        };
        std::env::remove_var(&key);

        assert!(
            status.success(),
            "child inherited parent env secret {key}; shell exited {status:?}",
        );
    }

    /// Positive control: the allow-list must still forward `PATH`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn child_inherits_path_from_allowlist() {
        let parent_path = std::env::var("PATH").unwrap_or_default();
        assert!(
            !parent_path.is_empty(),
            "test precondition: parent PATH must be non-empty"
        );

        let script = format!("test \"$PATH\" = {parent:?}", parent = parent_path);
        let mut child =
            ManagedStdioChild::spawn(cfg("/bin/sh", &["-c", &script])).expect("spawn sh path");

        let status = loop {
            let ev = tokio::time::timeout(std::time::Duration::from_secs(5), child.events.recv())
                .await
                .expect("recv timed out")
                .expect("channel closed before Exit");
            if let StdioChildEvent::Exit(s) = ev {
                break s;
            }
        };
        assert!(
            status.success(),
            "child PATH did not carry parent PATH; shell exited {status:?}",
        );
    }

    /// Caller-declared `extra_env` reaches the child after the allow-list
    /// scrub.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn extra_env_reaches_child_after_clear() {
        let mut extra = BTreeMap::new();
        extra.insert(
            "FORGE_CORE_EXTRA_VAR".to_string(),
            "declared-value".to_string(),
        );
        let mut cfg = cfg(
            "/bin/sh",
            &["-c", "test \"$FORGE_CORE_EXTRA_VAR\" = declared-value"],
        );
        cfg.extra_env = extra;

        let mut child = ManagedStdioChild::spawn(cfg).expect("spawn sh extra-env");
        let status = loop {
            let ev = tokio::time::timeout(std::time::Duration::from_secs(5), child.events.recv())
                .await
                .expect("recv timed out")
                .expect("channel closed before Exit");
            if let StdioChildEvent::Exit(s) = ev {
                break s;
            }
        };
        assert!(
            status.success(),
            "extra_env did not reach child; shell exited {status:?}",
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn spawn_error_surfaces_when_binary_missing() {
        let err = ManagedStdioChild::spawn(cfg("/nonexistent/forge-core-binary", &[]))
            .expect_err("missing binary must error");
        assert!(
            format!("{err:#}").contains("spawning stdio child"),
            "spawn error should mention the failed program: {err:#}",
        );
    }

    /// Stdout EOF with no frames must still surface a terminal `Exit`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn yields_exit_only_when_no_frames_emitted() {
        let mut child =
            ManagedStdioChild::spawn(cfg("/bin/sh", &["-c", "exit 7"])).expect("spawn sh exit 7");
        let ev = tokio::time::timeout(std::time::Duration::from_secs(5), child.events.recv())
            .await
            .expect("recv timed out")
            .expect("channel closed before Exit");
        match ev {
            StdioChildEvent::Exit(status) => {
                assert_eq!(status.code(), Some(7), "exit code must be surfaced");
            }
            other => panic!("expected Exit, got {other:?}"),
        }
    }

    /// F-347 / F-351 primitive contract: when `max_frame_bytes` is set,
    /// an over-cap stdout line surfaces as `MalformedStdout` and the
    /// reader keeps running so a well-formed frame following the over-cap
    /// line still reaches the channel.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn over_cap_stdout_emits_malformed_and_keeps_reader_alive() {
        // 6 KiB of 'x' (no newline), then a valid JSON frame. Tiny cap
        // (1 KiB) keeps the test fast.
        let script = r#"
            head -c 6144 /dev/zero | tr '\0' 'x'
            printf '\n{"ok":true}\n'
        "#;
        let mut cfg = cfg("/bin/sh", &["-c", script]);
        cfg.max_frame_bytes = Some(1024);
        let mut child = ManagedStdioChild::spawn(cfg).expect("spawn sh over-cap");

        let mut saw_malformed = false;
        let mut saw_valid = false;
        loop {
            let ev = tokio::time::timeout(std::time::Duration::from_secs(10), child.events.recv())
                .await
                .expect("recv timed out");
            match ev {
                Some(StdioChildEvent::MalformedStdout { bytes_discarded }) => {
                    assert!(bytes_discarded >= 1024, "discard >= cap");
                    saw_malformed = true;
                }
                Some(StdioChildEvent::Message(v)) => {
                    assert_eq!(v, serde_json::json!({"ok": true}));
                    saw_valid = true;
                }
                Some(StdioChildEvent::Exit(_)) | None => break,
                Some(_) => continue,
            }
        }
        assert!(saw_malformed, "expected MalformedStdout for over-cap line");
        assert!(saw_valid, "reader must survive and deliver next frame");
    }
}
