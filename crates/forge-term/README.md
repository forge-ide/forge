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
lives on the Rust side, the frontend runs a thin xterm.js renderer.
Raw PTY bytes are xterm.js-compatible by construction, so the byte
stream delivered to consumers is a direct pass-through (F-124).

With the **`ghostty-vt` cargo feature enabled** (F-146), the PTY reader
tees bytes into a dedicated driver thread that owns a
`libghostty_vt::Terminal`. VT state — cursor position, total/scrollback
rows, modes, title, etc. — is then queryable *authoritatively* from the
Rust side without parsing the stream ourselves:

- `TerminalSession::cursor_position() -> Result<CursorPosition, VtError>`
- `TerminalSession::total_rows() -> Result<usize, VtError>`
- `TerminalSession::scrollback_rows() -> Result<usize, VtError>`

The byte stream stays byte-identical to feature-off; the feature adds
query authority, not stream rewriting. Building the feature requires
`zig` on the host because `libghostty-vt-sys` vendor-fetches the
Ghostty C sources at build time.

## Further reading

- [Crate architecture — `forge-term`](../../docs/architecture/crate-architecture.md#37-forge-fs-forge-lsp-forge-term-forge-ipc-forge-cli-forge-shell)
- [Architecture overview — Terminal backend](../../docs/architecture/overview.md)
