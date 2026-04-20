# forge-term

Terminal backend for Forge. Spawns a child process under a PTY, forwards
raw PTY output to subscribers as a byte stream, and surfaces the final
process exit status as the last event on that stream. The emitted bytes
are xterm.js-compatible as-is — the frontend renderer (F-125) consumes
them unchanged.

One `TerminalSession` per pane. Dropping the handle SIGTERMs the child,
reaps the zombie, and delivers a final `TerminalEvent::Exit` so consumers
can surface exit codes in the UI.

## Role in the workspace

- Depended on by: `forge-shell` once the terminal pane lands in F-125.
- Depends on: `portable-pty` (PTY allocation + spawn + resize + kill),
  `tokio` (mpsc channel), `thiserror`.

## Key types / entry points

- `TerminalSession::spawn(ShellSpec, PathBuf, TerminalSize)` — returns
  `(TerminalSession, mpsc::Receiver<TerminalEvent>)`. The receiver is the
  "byte-stream receiver" called out in the architecture doc.
- `TerminalSession::write(&[u8])` — forward keystrokes / paste input back
  to the PTY master.
- `TerminalSession::resize(cols, rows)` — update PTY window size
  (delivers SIGWINCH to the child on Unix).
- `TerminalEvent` — `Bytes(Vec<u8>)` for PTY output, `Exit(ExitStatus)`
  as the final event.
- `Drop` — sends SIGTERM via `portable-pty`'s `ChildKiller`, joins the
  reaper + reader threads.

## Two-layer VT state

Forge commits to the two-layer terminal model in
`docs/architecture/overview.md` "Terminal backend" and
`docs/architecture/crate-architecture.md` §3.7: authoritative VT state
lives on the Rust side, the frontend runs a thin xterm.js renderer. Raw
PTY bytes are xterm.js-compatible by construction (xterm.js is itself a
VT state machine), so the current implementation forwards them directly.
Driving `ghostty-vt` as the authoritative state machine is staged as a
follow-up once the crate is pulled into CI with its zig/C toolchain
prerequisites.

## Further reading

- [Crate architecture — `forge-term`](../../docs/architecture/crate-architecture.md#37-forge-fs-forge-lsp-forge-term-forge-ipc-forge-cli-forge-shell)
- [Architecture overview — Terminal backend](../../docs/architecture/overview.md)
