# forge-shell

The Tauri 2 host binary that boots the Forge desktop GUI. Loads the Solid web bundle from `web/packages/app`, registers the Tauri command surface that the webview calls into, and brokers the IPC dance between the webview and any running `forged` session daemons. The crate is split so its pure pieces (window specs, dashboard helpers, IPC types) compile and unit-test on hosts without WebKitGTK; the live Tauri runtime sits behind the default `webview` feature.

## Role in the workspace

- Depended on by: nothing internal — this is a leaf binary.
- Depends on: `forge-core`, `forge-ipc`, `forge-providers`, `tauri` (gated), `tokio`.

## Key types / entry points

- `bin/forge-shell` (`src/main.rs`) — Tauri app entrypoint, gated on the `webview` feature.
- `window_spec` — pure declarative window configuration. Unit-tested with `--no-default-features`.
- `window_manager` — runtime adapter that applies a `WindowSpec` to a live `tauri::AppHandle` (gated on `webview`).
- `ipc` — the Tauri command set exposed to the webview, including the per-window-label authorisation helpers (gated on `webview`).
- `dashboard` / `dashboard_sessions` — provider-status probe with TTL cache and the dashboard sessions list / open commands (`collect_sessions`, `Pinger`).
- `bridge` — glue between the webview and the session UDS framing.

### Cargo features

- `webview` (default) — pulls the Tauri 2 runtime (`wry` + `webkit2gtk` on Linux).
- `webview-test` — also enables Tauri's `test` helpers (`mock_builder`, `WebviewWindowBuilder`) for integration tests.

## Further reading

- [Crate architecture — `forge-shell`](../../docs/architecture/crate-architecture.md#37-forge-fs-forge-lsp-forge-term-forge-ipc-forge-cli-forge-shell)
- [Window hierarchy](../../docs/architecture/window-hierarchy.md)
- [IPC contracts](../../docs/architecture/ipc-contracts.md)
- [Frontend architecture](../../docs/frontend/architecture.md)
