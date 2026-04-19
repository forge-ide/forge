# forge-term

Terminal emulator integration — wraps the `ghostty-vt` state machine and exposes a byte stream over IPC for the webview's xterm.js-compatible renderer. Reserved scaffold in Phase 1 — the crate compiles into the workspace as a placeholder so the eventual VT plumbing has a stable home. The Phase 1 GUI does not yet host a terminal pane.

## Role in the workspace

- Depended on by: nothing yet; future terminal-pane wiring in `forge-shell` will consume it.
- Depends on: nothing (intentionally empty until implementation begins).

## Key types / entry points

- _None yet._ The planned VT-state wrapper and IPC byte-stream surface are described in the architecture doc.

## Further reading

- [Crate architecture — `forge-term`](../../docs/architecture/crate-architecture.md#37-forge-fs-forge-lsp-forge-term-forge-ipc-forge-cli-forge-shell)
