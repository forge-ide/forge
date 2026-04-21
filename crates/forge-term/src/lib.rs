//! # forge-term
//!
//! Terminal backend for Forge. One [`TerminalSession`] per pane. Spawns a
//! child process under a PTY via [`portable_pty`], forwards raw PTY output
//! to subscribers as a byte stream, and surfaces the final process exit
//! status as the last event on that stream.
//!
//! The emitted bytes are xterm.js-compatible as-is: PTYs produce the same
//! VT escape sequences xterm.js parses natively. The frontend renderer
//! (F-125) consumes [`TerminalEvent::Bytes`] payloads unchanged.
//!
//! ## Two-layer VT state
//!
//! Forge's authoritative VT state lives on the Rust side (see
//! `docs/architecture/overview.md` "Terminal backend" and
//! `docs/architecture/crate-architecture.md` §3.7). With the
//! [`ghostty-vt`](#cargo-features) cargo feature enabled, the PTY reader tees
//! the byte stream into a dedicated driver thread that owns a
//! [`libghostty_vt::Terminal`]. The authoritative VT state is queryable via
//! [`TerminalSession::cursor_position`], [`TerminalSession::total_rows`],
//! and [`TerminalSession::scrollback_rows`] — answers come from the
//! ghostty-vt parser, not from the raw bytes. The byte stream delivered to
//! consumers is byte-identical with or without the feature so the xterm.js
//! renderer (F-125) behaves the same either way; the feature adds query
//! authority, not stream rewriting.
//!
//! ## Cargo features
//!
//! - `ghostty-vt` (off by default) — enables the ghostty-vt driver thread
//!   and the VT-state query methods. Building this feature requires `zig`
//!   on the host because the underlying sys crate vendor-fetches Ghostty C
//!   sources at build time.
//!
//! ## Lifecycle
//!
//! - `spawn(shell, cwd, size)` launches the child under a PTY and returns a
//!   handle plus an `mpsc::Receiver<TerminalEvent>` ("byte-stream receiver").
//! - `write(bytes)` sends user input back to the PTY.
//! - `resize(cols, rows)` adjusts PTY window size (SIGWINCH on Unix).
//! - `Drop` sends SIGTERM (via portable-pty's `ChildKiller`), reaps the
//!   child, and emits a final `TerminalEvent::Exit` before closing the
//!   receiver.
//!
//! ## Thread model
//!
//! `portable-pty`'s reader/writer are blocking `std::io` handles. We drive
//! them from dedicated OS threads and forward events to async consumers
//! via a tokio `mpsc` channel; the public API is async-friendly without
//! requiring callers to pin the session to a single thread. When the
//! `ghostty-vt` feature is enabled a third OS thread owns the ghostty-vt
//! [`Terminal`](libghostty_vt::Terminal) (which is `!Send + !Sync`) and
//! serializes every VT mutation and query through a command channel.

#![warn(missing_docs)]

use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use portable_pty::{native_pty_system, Child, ChildKiller, CommandBuilder, MasterPty, PtySize};
use tokio::sync::mpsc;

#[cfg(feature = "ghostty-vt")]
mod vt;
#[cfg(feature = "ghostty-vt")]
pub use vt::{CursorPosition, VtError};

/// Result alias for fallible [`TerminalSession`] operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors produced by [`TerminalSession`].
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Failed to allocate a PTY pair via the OS.
    #[error("pty allocation failed: {0}")]
    PtyAlloc(String),

    /// Failed to spawn the child process under the PTY.
    #[error("spawn failed: {0}")]
    Spawn(String),

    /// Failed to acquire the PTY reader (already taken or detached).
    #[error("pty reader unavailable: {0}")]
    Reader(String),

    /// Failed to acquire the PTY writer (already taken or detached).
    #[error("pty writer unavailable: {0}")]
    Writer(String),

    /// Writing user input to the PTY failed.
    #[error("pty write io error: {0}")]
    Io(#[from] std::io::Error),

    /// Resize syscall against the PTY failed.
    #[error("resize failed: {0}")]
    Resize(String),
}

/// Dimensions of the PTY window, in terminal cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalSize {
    /// Number of columns.
    pub cols: u16,
    /// Number of rows.
    pub rows: u16,
}

impl Default for TerminalSize {
    fn default() -> Self {
        Self { cols: 80, rows: 24 }
    }
}

impl From<TerminalSize> for PtySize {
    fn from(s: TerminalSize) -> Self {
        PtySize {
            cols: s.cols,
            rows: s.rows,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

/// Events emitted on the byte-stream receiver.
///
/// Consumers receive any number of [`TerminalEvent::Bytes`] followed by
/// exactly one terminal [`TerminalEvent::Exit`] after the child process
/// reaps, at which point the sender is dropped and the receiver yields
/// `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalEvent {
    /// Raw PTY output bytes. Pass through to xterm.js unchanged.
    Bytes(Vec<u8>),
    /// Process exit status. Always the final event before channel close.
    Exit(ExitStatus),
}

/// Process exit information.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitStatus {
    /// Exit code if the process exited normally; `None` if killed by signal
    /// (or otherwise unavailable — e.g. drop-initiated SIGTERM on some
    /// platforms).
    pub code: Option<i32>,
    /// `true` if the session terminated because of [`Drop`] (SIGTERM sent
    /// from this process). Distinguishes clean user exits from forced
    /// teardown in tests and diagnostics.
    pub killed_by_drop: bool,
}

/// Shell invocation spec used by [`TerminalSession::spawn`].
#[derive(Debug, Clone)]
pub struct ShellSpec {
    /// Executable to run (e.g. `/bin/sh`).
    pub program: OsString,
    /// Arguments to pass after the program name.
    pub args: Vec<OsString>,
}

impl ShellSpec {
    /// Build a shell spec that runs `program` with no additional arguments.
    pub fn new<S: Into<OsString>>(program: S) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
        }
    }

    /// Build a shell spec that runs `program` with the given `args`.
    pub fn with_args<S, I, A>(program: S, args: I) -> Self
    where
        S: Into<OsString>,
        I: IntoIterator<Item = A>,
        A: Into<OsString>,
    {
        Self {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
        }
    }
}

/// Active PTY-backed terminal session.
///
/// Constructed via [`TerminalSession::spawn`]. Keeps the child process
/// alive for as long as the handle is held; drops SIGTERM the child and
/// reap it on destruction.
pub struct TerminalSession {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child_killer: Box<dyn ChildKiller + Send + Sync>,
    // Reaper owns (and joins) the reader thread so `Exit` is guaranteed to
    // arrive after all `Bytes`. We only hold the reaper handle here.
    reaper_thread: Option<JoinHandle<()>>,
    // Shared flag: set by Drop so the reaper can tag the ExitStatus as
    // killed_by_drop before it sends the final event. Arc<Mutex<..>> is
    // the simplest shape that works across an OS thread boundary.
    drop_flag: Arc<Mutex<bool>>,
    /// Handle to the ghostty-vt driver thread + its command channel.
    /// Present iff the `ghostty-vt` feature is enabled — reader tees bytes
    /// into `vt.tx`; queries dispatch commands + block on oneshot replies.
    #[cfg(feature = "ghostty-vt")]
    vt: vt::VtHandle,
}

impl std::fmt::Debug for TerminalSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalSession")
            .field("reaper_thread", &self.reaper_thread.is_some())
            .finish()
    }
}

impl TerminalSession {
    /// Spawn `shell` under a fresh PTY sized to `size`, rooted at `cwd`.
    ///
    /// Returns the session handle and the byte-stream receiver. The
    /// receiver yields [`TerminalEvent::Bytes`] as the child writes, then
    /// a final [`TerminalEvent::Exit`] when the child reaps.
    pub fn spawn(
        shell: ShellSpec,
        cwd: PathBuf,
        size: TerminalSize,
    ) -> Result<(Self, mpsc::Receiver<TerminalEvent>)> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(size.into())
            .map_err(|e| Error::PtyAlloc(e.to_string()))?;

        let mut cmd = CommandBuilder::new(&shell.program);
        for arg in &shell.args {
            cmd.arg(arg);
        }
        cmd.cwd(cwd);

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| Error::Spawn(e.to_string()))?;

        let child_killer = child.clone_killer();

        // Drop the slave — the child inherited it. Holding it in the parent
        // prevents EOF propagation when the child exits.
        drop(pair.slave);

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| Error::Reader(e.to_string()))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| Error::Writer(e.to_string()))?;

        let (tx, rx) = mpsc::channel::<TerminalEvent>(128);

        // With ghostty-vt on, the reader tees into the driver thread which
        // owns the VT state machine. The driver is `!Send + !Sync` internally
        // so it lives on its own dedicated OS thread; the reader just pushes
        // bytes via a bounded channel. See `vt::spawn_vt_driver`.
        #[cfg(feature = "ghostty-vt")]
        let vt = vt::spawn_vt_driver(size.cols, size.rows);

        let reader_thread = spawn_reader_thread(
            reader,
            tx.clone(),
            #[cfg(feature = "ghostty-vt")]
            vt.tx.clone(),
        );
        let drop_flag = Arc::new(Mutex::new(false));
        // Reaper waits on child.wait(), then drains the reader so Exit is
        // always the last event the receiver sees. Without the join, a fast
        // child can reap before the reader pumps the last chunk.
        let reaper_thread = spawn_reaper_thread(child, tx, drop_flag.clone(), reader_thread);

        Ok((
            Self {
                master: pair.master,
                writer,
                child_killer,
                reaper_thread: Some(reaper_thread),
                drop_flag,
                #[cfg(feature = "ghostty-vt")]
                vt,
            },
            rx,
        ))
    }

    /// Write user input to the PTY. Bytes pass through unmodified.
    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()?;
        Ok(())
    }

    /// Resize the PTY window (delivers SIGWINCH to the child on Unix).
    ///
    /// When the `ghostty-vt` feature is enabled, the VT state machine is
    /// resized to the same dimensions so queries reflect the new geometry.
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        let size = PtySize {
            cols,
            rows,
            pixel_width: 0,
            pixel_height: 0,
        };
        self.master
            .resize(size)
            .map_err(|e| Error::Resize(e.to_string()))?;

        #[cfg(feature = "ghostty-vt")]
        {
            // Best-effort: a resize failure on the VT side must not mask a
            // successful PTY resize. We surface it separately.
            if let Err(e) = self.vt.resize(cols, rows) {
                tracing::warn!(error = ?e, "ghostty-vt resize failed");
            }
        }
        Ok(())
    }

    /// Get the authoritative cursor position as tracked by the ghostty-vt
    /// parser. Available only with the `ghostty-vt` feature enabled.
    ///
    /// The returned value reflects every VT sequence the PTY has emitted
    /// up to the time the reader thread handed it to the driver. Under
    /// heavy output it may lag the byte stream slightly; callers that need
    /// perfectly synchronous readings should drain the byte-stream receiver
    /// first.
    #[cfg(feature = "ghostty-vt")]
    pub fn cursor_position(&self) -> std::result::Result<CursorPosition, VtError> {
        self.vt.cursor_position()
    }

    /// Total rows (active + scrollback) in the authoritative VT grid.
    /// Available only with the `ghostty-vt` feature enabled.
    #[cfg(feature = "ghostty-vt")]
    pub fn total_rows(&self) -> std::result::Result<usize, VtError> {
        self.vt.total_rows()
    }

    /// Rows currently in the scrollback buffer (total rows minus viewport
    /// rows). Available only with the `ghostty-vt` feature enabled.
    #[cfg(feature = "ghostty-vt")]
    pub fn scrollback_rows(&self) -> std::result::Result<usize, VtError> {
        self.vt.scrollback_rows()
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        // Mark the teardown as drop-initiated so the reaper tags the
        // final ExitStatus accordingly.
        if let Ok(mut guard) = self.drop_flag.lock() {
            *guard = true;
        }

        // SIGTERM on Unix; TerminateProcess on Windows.
        let _ = self.child_killer.kill();

        // Join the reaper thread. The reaper itself joins the reader after
        // the child reaps, so both workers drain before we return.
        if let Some(handle) = self.reaper_thread.take() {
            let _ = handle.join();
        }

        // Tear down the VT driver after the reader thread has drained so
        // any final bytes reach the state machine before we ask it to exit.
        #[cfg(feature = "ghostty-vt")]
        self.vt.shutdown();
    }
}

fn spawn_reader_thread(
    mut reader: Box<dyn Read + Send>,
    tx: mpsc::Sender<TerminalEvent>,
    #[cfg(feature = "ghostty-vt")] vt_tx: std::sync::mpsc::SyncSender<vt::VtCommand>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break, // EOF: PTY closed, child exited
                Ok(n) => {
                    let chunk = buf[..n].to_vec();

                    // Tee to the VT driver *first*: the driver's job is to
                    // keep its state machine in sync with the stream, and
                    // in-order delivery matters. A dropped send on the VT
                    // side is fatal for queries but not for rendering, so
                    // we don't abort the public channel if it fails.
                    #[cfg(feature = "ghostty-vt")]
                    {
                        let _ = vt_tx.send(vt::VtCommand::Bytes(chunk.clone()));
                    }

                    // blocking_send: we're on a dedicated OS thread.
                    if tx.blocking_send(TerminalEvent::Bytes(chunk)).is_err() {
                        // Receiver dropped; no one listening.
                        break;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break, // Read error: bail out cleanly.
            }
        }
    })
}

fn spawn_reaper_thread(
    mut child: Box<dyn Child + Send + Sync>,
    tx: mpsc::Sender<TerminalEvent>,
    drop_flag: Arc<Mutex<bool>>,
    reader_thread: JoinHandle<()>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let status = child.wait();
        // Wait for the reader thread to drain any remaining PTY output
        // (reader exits on EOF once the child closes its PTY slave). This
        // guarantees Exit is the *last* event delivered on the channel.
        let _ = reader_thread.join();
        let killed_by_drop = drop_flag.lock().map(|g| *g).unwrap_or(false);
        let code = status.ok().map(|s| s.exit_code() as i32);
        let exit = ExitStatus {
            code,
            killed_by_drop,
        };
        // The send may fail if the receiver is already gone; that's fine.
        let _ = tx.blocking_send(TerminalEvent::Exit(exit));
    })
}

/// Helper for tests: drain `rx` until an `Exit` event arrives or `timeout`
/// elapses. Returns the collected byte payload (concatenated) and the
/// final exit status if one was observed.
#[doc(hidden)]
pub async fn drain_until_exit(
    rx: &mut mpsc::Receiver<TerminalEvent>,
    timeout: Duration,
) -> (Vec<u8>, Option<ExitStatus>) {
    let mut bytes = Vec::new();
    let mut exit = None;
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Some(TerminalEvent::Bytes(b))) => bytes.extend_from_slice(&b),
            Ok(Some(TerminalEvent::Exit(e))) => {
                exit = Some(e);
                break;
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }
    (bytes, exit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn spawn_and_echo_round_trip() {
        // DoD: spawn returns a handle + byte-stream receiver; bytes flow.
        let cwd = std::env::temp_dir();
        let (session, mut rx) = TerminalSession::spawn(
            ShellSpec::with_args("/bin/sh", ["-c", "printf FORGE_TERM_OK"]),
            cwd,
            TerminalSize::default(),
        )
        .expect("spawn");

        let (bytes, exit) = drain_until_exit(&mut rx, Duration::from_secs(5)).await;

        // We should have seen the child's stdout on the stream.
        assert!(
            bytes
                .windows(b"FORGE_TERM_OK".len())
                .any(|w| w == b"FORGE_TERM_OK"),
            "expected FORGE_TERM_OK in stream; got: {:?}",
            String::from_utf8_lossy(&bytes)
        );
        let exit = exit.expect("expected Exit event before timeout");
        assert!(
            !exit.killed_by_drop,
            "child exited naturally; should not be killed_by_drop"
        );
        assert_eq!(exit.code, Some(0), "printf should exit 0");

        // Drop the session cleanly (child already reaped).
        drop(session);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn write_input_reaches_child() {
        // DoD: TerminalSession::write forwards bytes to the PTY.
        // Use `sh -c "sleep 0.2; read X; printf GOT=%s $X"` so the child
        // is definitively blocked on `read` before we write. Without the
        // grace period the write can race the shell's startup and vanish.
        let cwd = std::env::temp_dir();
        let (mut session, mut rx) = TerminalSession::spawn(
            ShellSpec::with_args(
                "/bin/sh",
                ["-c", "sleep 0.2; read x; printf 'GOT=%s' \"$x\""],
            ),
            cwd,
            TerminalSize::default(),
        )
        .expect("spawn");

        // Give the child time to block on `read` before we deliver input.
        tokio::time::sleep(Duration::from_millis(400)).await;
        session.write(b"hi\n").expect("write");

        let (bytes, exit) = drain_until_exit(&mut rx, Duration::from_secs(5)).await;
        let s = String::from_utf8_lossy(&bytes);
        assert!(
            s.contains("GOT=hi"),
            "expected GOT=hi in stream; got: {s:?}"
        );
        assert!(exit.is_some(), "expected Exit event");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn resize_updates_pty_dimensions() {
        // DoD: TerminalSession::resize. Verify by querying `stty size`
        // after the resize — the child sees the new window size.
        let cwd = std::env::temp_dir();
        let (mut session, mut rx) = TerminalSession::spawn(
            // Wait briefly for the resize to land, then print tty size.
            ShellSpec::with_args(
                "/bin/sh",
                ["-c", "sleep 0.2; stty size 2>/dev/null || printf 'NOSTTY'"],
            ),
            cwd,
            TerminalSize { cols: 80, rows: 24 },
        )
        .expect("spawn");

        session.resize(132, 50).expect("resize");

        let (bytes, exit) = drain_until_exit(&mut rx, Duration::from_secs(5)).await;
        let s = String::from_utf8_lossy(&bytes);
        // stty size prints "<rows> <cols>".
        assert!(
            s.contains("50 132") || s.contains("NOSTTY"),
            "expected '50 132' (rows cols) in stream; got: {s:?}"
        );
        assert!(exit.is_some(), "expected Exit event");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn drop_terminates_child_and_emits_exit() {
        // DoD: Drop SIGTERMs the child and surfaces exit via final event.
        let cwd = std::env::temp_dir();
        let (session, mut rx) = TerminalSession::spawn(
            // Long-running child we'll kill via Drop.
            ShellSpec::with_args("/bin/sh", ["-c", "sleep 30"]),
            cwd,
            TerminalSize::default(),
        )
        .expect("spawn");

        // Let the child start.
        tokio::time::sleep(Duration::from_millis(100)).await;

        drop(session);

        // After drop, the receiver must surface a final Exit event.
        let mut saw_exit = false;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, rx.recv()).await {
                Ok(Some(TerminalEvent::Exit(e))) => {
                    assert!(
                        e.killed_by_drop,
                        "Drop-initiated teardown must tag killed_by_drop"
                    );
                    saw_exit = true;
                    break;
                }
                Ok(Some(TerminalEvent::Bytes(_))) => continue,
                Ok(None) => break,
                Err(_) => break,
            }
        }
        assert!(saw_exit, "expected final Exit event after Drop");
    }

    #[test]
    fn terminal_size_default_is_80x24() {
        // Sanity: the conventional default matches POSIX.
        assert_eq!(TerminalSize::default(), TerminalSize { cols: 80, rows: 24 });
    }

    #[test]
    fn shell_spec_constructors() {
        // Basic constructor ergonomics for callers building ShellSpecs.
        let s = ShellSpec::new("/bin/sh");
        assert!(s.args.is_empty());

        let s2 = ShellSpec::with_args("/bin/sh", ["-c", "echo hi"]);
        assert_eq!(s2.args.len(), 2);
    }

    // ----------------------------------------------------------------------
    // F-146: VT state authority on the Rust side.
    //
    // These tests are gated on `ghostty-vt` because the feature pulls in a
    // C/zig build dep that isn't universally available. With the feature
    // off the existing F-124 tests above still define the pass-through
    // contract — those MUST stay green either way.
    // ----------------------------------------------------------------------

    #[cfg(feature = "ghostty-vt")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn vt_state_reflects_child_output_via_ghostty_vt() {
        // DoD (F-146): "New test verifies VT state authority on the Rust
        // side — e.g., a cursor_position() or scrollback_lines() query API
        // on TerminalSession returns values driven by ghostty-vt's parser."
        //
        // We spawn a shell that prints exactly five characters and exits.
        // After the byte stream settles, TerminalSession::cursor_position()
        // must report col=5/row=0, proving the VT driver — not the raw PTY
        // stream — is the source of truth.
        let cwd = std::env::temp_dir();
        let (session, mut rx) = TerminalSession::spawn(
            ShellSpec::with_args("/bin/sh", ["-c", "printf hello"]),
            cwd,
            TerminalSize::default(),
        )
        .expect("spawn");

        // Drain until the child exits so we know the driver has seen every
        // byte the child produced.
        let (bytes, exit) = drain_until_exit(&mut rx, Duration::from_secs(5)).await;
        assert!(
            bytes.windows(b"hello".len()).any(|w| w == b"hello"),
            "expected child output in byte stream (pass-through invariant)"
        );
        assert!(exit.is_some(), "child must reap before we query");

        // Give the VT driver a beat to drain its own channel after the
        // PTY reader pushed the final chunk — they're independent threads.
        for _ in 0..20 {
            if let Ok(pos) = session.cursor_position() {
                if pos.col == 5 {
                    return; // pass
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let pos = session
            .cursor_position()
            .expect("cursor_position must succeed while session is alive");
        assert_eq!(
            pos,
            CursorPosition { col: 5, row: 0 },
            "ghostty-vt must be authoritative after `printf hello`"
        );
    }

    #[cfg(feature = "ghostty-vt")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn vt_query_before_any_output_returns_origin() {
        // DoD (F-146): the query API is usable from the moment the session
        // exists — not just after we've seen bytes. Spawns a `sleep`, asks
        // for the cursor immediately, expects (0, 0).
        let cwd = std::env::temp_dir();
        let (session, _rx) = TerminalSession::spawn(
            ShellSpec::with_args("/bin/sh", ["-c", "sleep 2"]),
            cwd,
            TerminalSize::default(),
        )
        .expect("spawn");

        let pos = session
            .cursor_position()
            .expect("cursor_position must succeed on a live session");
        assert_eq!(
            pos,
            CursorPosition { col: 0, row: 0 },
            "fresh VT state starts at origin"
        );

        // Before the sleep finishes, total_rows must match the viewport
        // height (24 cells) and scrollback must be zero.
        assert_eq!(session.total_rows().expect("total_rows"), 24);
        assert_eq!(session.scrollback_rows().expect("scrollback_rows"), 0);
    }
}
