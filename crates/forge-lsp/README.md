# forge-lsp

Language-server *management* — the layer that downloads, updates, and tracks the default LSP servers Forge bundles for first-use language support. Reserved scaffold in Phase 1 — the crate compiles into the workspace as a placeholder. As the architecture doc notes, the actual LSP client runs in the webview via `monaco-languageclient`, so this crate is deliberately a lifecycle / install layer, not an LSP proxy.

## Role in the workspace

- Depended on by: nothing yet; future webview wiring will trigger downloads through it.
- Depends on: nothing (intentionally empty until implementation begins).

## Key types / entry points

- _None yet._ The planned shape (per-language download/update jobs, version tracking, install path resolution) is described in the architecture doc.

## Further reading

- [Crate architecture — `forge-lsp`](../../docs/architecture/crate-architecture.md#37-forge-fs-forge-lsp-forge-term-forge-ipc-forge-cli-forge-shell)
