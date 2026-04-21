# monaco-host

Isolated iframe app that hosts the Monaco editor and `monaco-languageclient`. Loaded by the parent `app` as `<iframe src="/monaco-host.html">` so Monaco's AMD loader, web workers, and global scope don't pollute the Solid tree. Communicates with the parent via `window.postMessage`; never talks to Tauri directly.

## Role in the workspace

- Depended on by: nothing — the parent `app` loads `dist/index.html` at runtime. No TypeScript import path crosses the package boundary.
- Depends on: `monaco-editor`, `monaco-languageclient`, `vscode-languageclient`, `vscode-ws-jsonrpc`, `vscode-jsonrpc`. Dev: `vitest`, `jsdom`, `typescript`, `vite`.

## postMessage protocol

All messages use a discriminated union on `kind`. The full TypeScript union is in `src/protocol.ts`; the summary below is the contract other packages rely on.

### Parent -> iframe

| `kind` | Payload | Effect |
| --- | --- | --- |
| `open` | `{ uri, languageId, value }` | Replace buffer, remember `uri` as active. |
| `close` | `{ uri }` | If `uri` is active, clear the buffer and forget it. |
| `save` | `{ uri }` | If `uri` is active, emit `save` back with current contents. |
| `focus` | `{}` | Focus the editor. No reply. |
| `client.message` | `{ payload }` | Deliver an LSP response/notification to the in-iframe language client. |

### Iframe -> parent

| `kind` | Payload | Meaning |
| --- | --- | --- |
| `ready` | `{}` | Emitted once on boot after the protocol is wired. |
| `opened` | `{ uri }` | Acknowledges an `open`. |
| `closed` | `{ uri }` | Acknowledges a `close`. |
| `save` | `{ uri, value }` | User/parent requested save; `value` is the current buffer. |
| `change` | `{ uri, value }` | Every buffer edit, full post-change contents. |
| `client.message` | `{ payload }` | Outbound LSP request/response (parent relays to `forge-lsp`, awaits reply). |
| `client.notification` | `{ payload }` | Outbound LSP notification (fire-and-forget; no reply bookkeeping). |

Outbound split rule (JSON-RPC 2.0): payloads with a `method` string and no `id` are classified as `client.notification`; anything else is `client.message`. Inbound `client.message` and `client.notification` are accepted symmetrically so the parent can forward server-initiated notifications such as `window/logMessage`.

The iframe signals readiness with `ready`; the parent should queue `open` messages until it arrives.

### LSP lifecycle

`MonacoLanguageClient` is instantiated at boot but **not started**. The parent webview drives the lifecycle via the F-123 IPC surface in `crates/forge-shell/src/ipc.rs` (PR #286):

1. Parent calls `lsp_start` to launch a `forge-lsp`-supervised server for the active language.
2. Parent tells the iframe the language is ready so the client can `.start()`. Outbound frames flow as iframe `client.message` / `client.notification` → parent → `lsp_send`; inbound frames flow as the `lsp_message` Tauri event → parent → iframe `client.message`.
3. Parent calls `lsp_stop` on teardown (owner-label authz rejects cross-session stops).

When no parent has started a server the language client is idle: the postMessage transport is still wired so the construction path is verified on every boot, but no LSP traffic flows.

## Test harness

The smoke test (`tests/protocol.test.ts`) imports **only** `src/protocol.ts`. That file deliberately has no transitive dependency on `monaco-editor` or `monaco-languageclient` — both pull in workers, AMD globals, and the `@codingame/monaco-vscode-*` peer graph, none of which are jsdom-safe.

To add coverage that exercises Monaco, write a Playwright test in the parent `app` package (outside this PR's scope) and boot the built iframe via its Vite preview server. Do not try to mount Monaco in Vitest.

## Theme drift

`src/theme.ts` duplicates hex values from `web/packages/design/src/tokens.css`. `scripts/check-tokens.mjs` does not cover this file — Monaco's `defineTheme` needs literal color strings, not CSS variables. When tokens change, update both in the same PR.

## Key types / entry points

- `src/index.ts` — Vite entry; mounts Monaco, wires the protocol, constructs the language client.
- `src/protocol.ts` — jsdom-safe. Message types, `IframeSocket` (a postMessage-backed `IWebSocket`), and `createIframeProtocol`.
- `src/editor.ts` — Monaco mount + `EditorLike` adapter.
- `src/client.ts` — `MonacoLanguageClient` construction wired to `IframeSocket` through `vscode-ws-jsonrpc`.
- `src/theme.ts` — Forge Ember theme for Monaco.
- Scripts: `pnpm dev` (Vite, port 5174), `pnpm build`, `pnpm test` (Vitest smoke), `pnpm typecheck`.

## Further reading

- [Build approach — Editor: Monaco in iframe](../../../docs/build/approach.md)
- [Architecture overview §7](../../../docs/architecture/overview.md)
- [Frontend architecture §9.3](../../../docs/frontend/architecture.md) — parent-relay LSP model matching this package's postMessage contract.
- [Color system](../../../docs/design/color-system.md)
