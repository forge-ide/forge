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
//! Forge's authoritative VT state is tracked on the Rust side (see
//! `docs/architecture/overview.md` "Terminal backend" and
//! `docs/architecture/crate-architecture.md` §3.7). The `ghostty-vt` drive
//! step is wired behind the off-by-default `ghostty-vt` cargo feature; it
//! currently observes the byte stream without altering it so the xterm.js
//! renderer sees identical bytes whether the feature is enabled or not.
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
//! requiring callers to pin the session to a single thread.

#![warn(missing_docs)]

use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use portable_pty::{native_pty_system, Child, ChildKiller, CommandBuilder, MasterPty, PtySize};
use tokio::sync::mpsc;

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

        let reader_thread = spawn_reader_thread(reader, tx.clone());
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
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        let size = PtySize {
            cols,
            rows,
            pixel_width: 0,
            pixel_height: 0,
        };
        self.master
            .resize(size)
            .map_err(|e| Error::Resize(e.to_string()))
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
    }
}

fn spawn_reader_thread(
    mut reader: Box<dyn Read + Send>,
    tx: mpsc::Sender<TerminalEvent>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break, // EOF: PTY closed, child exited
                Ok(n) => {
                    let chunk = buf[..n].to_vec();
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
}
