//! libghostty-vt integration (F-146).
//!
//! The ghostty-vt [`Terminal`](libghostty_vt::Terminal) is `!Send + !Sync`,
//! so it must live on a single dedicated OS thread. This module owns that
//! thread and exposes the terminal indirectly through a command channel:
//!
//! - [`VtCommand::Bytes`] — teed from the PTY reader; the driver parses
//!   them into VT state via `Terminal::vt_write`.
//! - [`VtCommand::Resize`] — propagated from [`crate::TerminalSession::resize`]
//!   so the VT grid stays the same size as the PTY.
//! - [`VtCommand::Query`] — a one-shot query (e.g. cursor position); the
//!   driver answers via the supplied [`std::sync::mpsc::SyncSender`] reply
//!   channel.
//! - [`VtCommand::Shutdown`] — asks the driver to drop its terminal and
//!   exit; sent from [`crate::TerminalSession`]'s `Drop`.
//!
//! All commands flow through a `std::sync::mpsc::sync_channel` with a small
//! bound so the PTY reader thread cannot runaway-fill memory if the VT
//! parser stalls. `send` is best-effort from the reader; queries are
//! `blocking_send` and will propagate a `DriverGone` error if the channel
//! closes.

use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::thread::JoinHandle;

use libghostty_vt::{Terminal, TerminalOptions};

/// Default scrollback budget for sessions created via
/// [`crate::TerminalSession::spawn`]. Matches the typical xterm.js
/// `scrollback: 10_000` default so the two layers agree.
const DEFAULT_MAX_SCROLLBACK: usize = 10_000;

/// Channel bound for the reader → VT driver stream. Large enough to
/// absorb a full-screen repaint (~80×24 cells plus escapes) without
/// reader stalls, small enough that a stuck driver can't bloat memory.
const VT_COMMAND_CHANNEL_BOUND: usize = 256;

/// Zero-indexed cursor position as tracked by the ghostty-vt parser.
/// Columns grow left-to-right; rows grow top-to-bottom within the
/// active (non-scrollback) area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorPosition {
    /// Zero-indexed column.
    pub col: u16,
    /// Zero-indexed row within the active area.
    pub row: u16,
}

/// Errors surfaced from VT-state queries.
#[derive(Debug, thiserror::Error)]
pub enum VtError {
    /// The driver thread is no longer reachable. This happens if the
    /// session has already been dropped or the driver panicked.
    #[error("vt driver channel closed")]
    DriverGone,

    /// The driver returned an error from the underlying libghostty-vt
    /// call. The message is whatever libghostty-vt's `Display` impl
    /// produced — opaque from our side.
    #[error("vt driver error: {0}")]
    Driver(String),
}

/// Kinds of one-shot queries the driver answers.
#[derive(Debug)]
pub(crate) enum QueryKind {
    CursorPosition,
    TotalRows,
    ScrollbackRows,
}

/// Reply payload for a query. Each variant pairs with exactly one
/// [`QueryKind`] so the caller can `match` on it after recv.
#[derive(Debug)]
pub(crate) enum QueryResponse {
    CursorPosition(Result<CursorPosition, VtError>),
    TotalRows(Result<usize, VtError>),
    ScrollbackRows(Result<usize, VtError>),
}

/// Commands sent to the VT driver thread. All variants are processed in
/// the order they arrive — `Bytes` must stay strictly ordered vs. queries
/// so answers reflect the latest parsed state.
#[derive(Debug)]
pub(crate) enum VtCommand {
    /// A chunk of PTY output to feed into the parser.
    Bytes(Vec<u8>),
    /// New terminal dimensions in cells.
    Resize { cols: u16, rows: u16 },
    /// One-shot query. Reply arrives via the sender.
    Query(QueryKind, SyncSender<QueryResponse>),
    /// Shut down the driver; the thread exits after draining pending
    /// commands.
    Shutdown,
}

/// Owner-side handle for the VT driver thread. Held by
/// [`crate::TerminalSession`]; `shutdown` is called from `Drop`.
pub(crate) struct VtHandle {
    pub(crate) tx: SyncSender<VtCommand>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for VtHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VtHandle")
            .field("thread", &self.thread.is_some())
            .finish()
    }
}

impl VtHandle {
    /// Send a synchronous resize to the driver. Returns `DriverGone` if
    /// the thread is no longer running.
    pub(crate) fn resize(&self, cols: u16, rows: u16) -> Result<(), VtError> {
        self.tx
            .send(VtCommand::Resize { cols, rows })
            .map_err(|_| VtError::DriverGone)
    }

    /// Query the current cursor position.
    pub(crate) fn cursor_position(&self) -> Result<CursorPosition, VtError> {
        let (reply_tx, reply_rx) = sync_channel(1);
        self.tx
            .send(VtCommand::Query(QueryKind::CursorPosition, reply_tx))
            .map_err(|_| VtError::DriverGone)?;
        match reply_rx.recv().map_err(|_| VtError::DriverGone)? {
            QueryResponse::CursorPosition(r) => r,
            _ => Err(VtError::Driver(
                "unexpected response variant for CursorPosition".into(),
            )),
        }
    }

    /// Total rows including scrollback.
    pub(crate) fn total_rows(&self) -> Result<usize, VtError> {
        let (reply_tx, reply_rx) = sync_channel(1);
        self.tx
            .send(VtCommand::Query(QueryKind::TotalRows, reply_tx))
            .map_err(|_| VtError::DriverGone)?;
        match reply_rx.recv().map_err(|_| VtError::DriverGone)? {
            QueryResponse::TotalRows(r) => r,
            _ => Err(VtError::Driver(
                "unexpected response variant for TotalRows".into(),
            )),
        }
    }

    /// Rows in the scrollback buffer only.
    pub(crate) fn scrollback_rows(&self) -> Result<usize, VtError> {
        let (reply_tx, reply_rx) = sync_channel(1);
        self.tx
            .send(VtCommand::Query(QueryKind::ScrollbackRows, reply_tx))
            .map_err(|_| VtError::DriverGone)?;
        match reply_rx.recv().map_err(|_| VtError::DriverGone)? {
            QueryResponse::ScrollbackRows(r) => r,
            _ => Err(VtError::Driver(
                "unexpected response variant for ScrollbackRows".into(),
            )),
        }
    }

    /// Signal the driver to exit, then join its thread. Best-effort — a
    /// missed signal or panicked thread is logged but not propagated (we
    /// are typically called from `Drop`).
    pub(crate) fn shutdown(&mut self) {
        let _ = self.tx.send(VtCommand::Shutdown);
        if let Some(handle) = self.thread.take() {
            if let Err(e) = handle.join() {
                tracing::warn!(?e, "vt driver thread panicked during shutdown");
            }
        }
    }
}

/// Spawn the VT driver thread sized to the initial PTY geometry.
///
/// The driver runs the standard `recv` loop: it owns a
/// [`Terminal`](libghostty_vt::Terminal) (so it must stay single-threaded)
/// and services [`VtCommand`]s until it receives `Shutdown` or the sender
/// drops.
pub(crate) fn spawn_vt_driver(cols: u16, rows: u16) -> VtHandle {
    let (tx, rx) = sync_channel::<VtCommand>(VT_COMMAND_CHANNEL_BOUND);
    let thread = std::thread::Builder::new()
        .name("forge-term-vt".into())
        .spawn(move || run_driver(rx, cols, rows))
        .expect("spawn vt driver thread");
    VtHandle {
        tx,
        thread: Some(thread),
    }
}

/// The driver loop. Owns the [`Terminal`] and never lets it cross threads.
fn run_driver(rx: Receiver<VtCommand>, cols: u16, rows: u16) {
    let mut terminal = match Terminal::new(TerminalOptions {
        cols,
        rows,
        max_scrollback: DEFAULT_MAX_SCROLLBACK,
    }) {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(error = ?e, "failed to construct libghostty-vt Terminal; driver exiting");
            // Drain incoming commands so senders don't block, but answer
            // every query with DriverGone-shaped errors. We achieve the
            // same effect by simply returning here: all subsequent sends
            // either succeed into the dropped rx (error suppressed on the
            // reader tee) or fail fast via SendError on queries.
            return;
        }
    };

    while let Ok(cmd) = rx.recv() {
        match cmd {
            VtCommand::Bytes(data) => {
                terminal.vt_write(&data);
            }
            VtCommand::Resize { cols, rows } => {
                if let Err(e) = terminal.resize(cols, rows, 0, 0) {
                    tracing::warn!(error = ?e, "libghostty-vt resize failed");
                }
            }
            VtCommand::Query(kind, reply) => {
                let response = match kind {
                    QueryKind::CursorPosition => {
                        let result = (|| -> Result<CursorPosition, VtError> {
                            let col = terminal
                                .cursor_x()
                                .map_err(|e| VtError::Driver(format!("{e:?}")))?;
                            let row = terminal
                                .cursor_y()
                                .map_err(|e| VtError::Driver(format!("{e:?}")))?;
                            Ok(CursorPosition { col, row })
                        })();
                        QueryResponse::CursorPosition(result)
                    }
                    QueryKind::TotalRows => {
                        let result = terminal
                            .total_rows()
                            .map_err(|e| VtError::Driver(format!("{e:?}")));
                        QueryResponse::TotalRows(result)
                    }
                    QueryKind::ScrollbackRows => {
                        let result = terminal
                            .scrollback_rows()
                            .map_err(|e| VtError::Driver(format!("{e:?}")));
                        QueryResponse::ScrollbackRows(result)
                    }
                };
                // Best-effort: if the caller dropped the reply channel
                // (e.g. timed out), we just discard the answer.
                let _ = reply.send(response);
            }
            VtCommand::Shutdown => break,
        }
    }
    // Terminal drops here; libghostty-vt releases its C resources.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn driver_tracks_cursor_on_plain_bytes() {
        let handle = spawn_vt_driver(80, 24);
        handle
            .tx
            .send(VtCommand::Bytes(b"hello".to_vec()))
            .expect("send bytes");
        let pos = handle.cursor_position().expect("query cursor");
        assert_eq!(pos, CursorPosition { col: 5, row: 0 });
    }

    #[test]
    fn driver_resize_reflects_in_state() {
        let handle = spawn_vt_driver(80, 24);
        handle.resize(132, 50).expect("resize");
        // Write one character so the cursor is on the new grid.
        handle
            .tx
            .send(VtCommand::Bytes(b"x".to_vec()))
            .expect("send bytes");
        let pos = handle.cursor_position().expect("query cursor");
        assert_eq!(pos.col, 1);
    }

    #[test]
    fn driver_shutdown_joins_cleanly() {
        let mut handle = spawn_vt_driver(80, 24);
        handle.shutdown();
        // After shutdown the thread handle is consumed; a second query
        // must report DriverGone rather than hang.
        let err = handle
            .cursor_position()
            .expect_err("query after shutdown must error");
        assert!(matches!(err, VtError::DriverGone));
    }

    #[test]
    fn cursor_newline_advances_row() {
        // Minimal VT sequence: two characters + LF → row 1, col 0.
        let handle = spawn_vt_driver(80, 24);
        handle
            .tx
            .send(VtCommand::Bytes(b"ab\r\n".to_vec()))
            .expect("send bytes");
        let pos = handle.cursor_position().expect("query cursor");
        assert_eq!(pos, CursorPosition { col: 0, row: 1 });
    }

    #[test]
    fn scrollback_is_bounded_by_default_max() {
        // Regression guard for F-383: drive more newlines than the scrollback
        // cap through the VT and assert the parser never reports a buffer
        // larger than `DEFAULT_MAX_SCROLLBACK`, nor a `total_rows` that
        // exceeds `viewport + cap`. Catches two classes of regression:
        //   (1) option plumbing: `max_scrollback` silently stops reaching
        //       `TerminalOptions`, causing unbounded growth.
        //   (2) ghostty-vt accounting: a future bump to libghostty-vt changes
        //       the semantics of `scrollback_rows`/`total_rows` and our cap
        //       stops being respected.
        //
        // Hermetic — writes the bytes directly through the driver channel;
        // no PTY or child process.
        const COLS: u16 = 80;
        const VIEWPORT_ROWS: u16 = 24;
        // Well past the cap so any unbounded growth would blow through it.
        let lines_to_write = DEFAULT_MAX_SCROLLBACK + 2_000;

        let handle = spawn_vt_driver(COLS, VIEWPORT_ROWS);

        // Send in bounded chunks so a single oversized `Vec<u8>` can't
        // stall the `VT_COMMAND_CHANNEL_BOUND`-sized command channel.
        let chunk_lines = 500;
        let mut remaining = lines_to_write;
        while remaining > 0 {
            let n = remaining.min(chunk_lines);
            let mut buf = Vec::with_capacity(n * 3);
            for _ in 0..n {
                buf.extend_from_slice(b"x\r\n");
            }
            handle
                .tx
                .send(VtCommand::Bytes(buf))
                .expect("send bytes chunk");
            remaining -= n;
        }

        let scrollback = handle.scrollback_rows().expect("query scrollback_rows");
        let total = handle.total_rows().expect("query total_rows");

        // Primary invariant from the DoD.
        assert!(
            scrollback <= DEFAULT_MAX_SCROLLBACK,
            "scrollback_rows={scrollback} exceeded cap DEFAULT_MAX_SCROLLBACK={DEFAULT_MAX_SCROLLBACK}",
        );
        assert!(
            total <= DEFAULT_MAX_SCROLLBACK + VIEWPORT_ROWS as usize,
            "total_rows={total} exceeded viewport+cap ({})",
            DEFAULT_MAX_SCROLLBACK + VIEWPORT_ROWS as usize,
        );
        // Sanity: rows left the viewport, so scrollback must have grown.
        // If this asserts `== 0` we know the byte stream was swallowed or
        // the parser never advanced rows — a different regression mode.
        assert!(
            scrollback > 0,
            "scrollback_rows=0 after writing {lines_to_write} lines — parser never scrolled",
        );
    }
}
