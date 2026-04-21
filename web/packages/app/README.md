# app

The Forge desktop UI: a Solid + Vite + TypeScript single-page app loaded by the `forge-shell` Tauri host. Defines the route tree (`/` → Dashboard, `/session/:id` → SessionWindow), wires the in-process Solid signal stores, and talks to the Rust side through the typed bindings re-exported from `@forge/ipc`. Component styling pulls CSS variables from `@forge/design`'s tokens; component behaviour is exercised by Vitest unit tests and Playwright end-to-end tests.

## Role in the workspace

- Depended on by: nothing — this is the leaf web app the Tauri host loads.
- Depends on: `@forge/design` (CSS tokens), `@forge/ipc` (typed wire types), `@solidjs/router`, `@tauri-apps/api`, `solid-js`. Dev: `vitest`, `@solidjs/testing-library`, `@playwright/test`, `vite-plugin-solid`.

## Key types / entry points

- `src/index.tsx` — Vite entry; mounts `<App />` into `#root`.
- `src/App.tsx` — root component; declares the `<Router>` with `Dashboard` and `SessionWindow` routes.
- `src/routes/` — per-route Solid components (`Dashboard`, `Session/SessionWindow`).
- `src/components/` — reusable Solid components.
- `src/stores/` — module-level Solid signals (active session, sessions store, catalog state).
- `src/ipc/` — thin wrappers over `@tauri-apps/api/core` `invoke` calls, typed against `@forge/ipc`.
- `src/lib/` — pure helpers shared across routes and components.
- `src/styles/` — app-level CSS that consumes `@forge/design`'s tokens.
- Scripts: `pnpm dev` (Vite dev server), `pnpm build`, `pnpm test` (Vitest), `pnpm test:e2e` (Playwright), `pnpm typecheck`.

## Monaco iframe (F-122)

The `EditorPane` component embeds the `monaco-host` iframe (F-121) at `/monaco-host/index.html`. That package builds in isolation (separate Vite config with `base: './'` so emitted asset URLs are relative) and is wired into this app via:

- `pnpm predev` / `pnpm prebuild` — build `monaco-host` and run `scripts/sync-monaco-host.mjs`
- `scripts/sync-monaco-host.mjs` — copies `web/packages/monaco-host/dist/` into `web/packages/app/public/monaco-host/`

Vite serves `public/*` at the bundle root, so `/monaco-host/index.html` resolves in all three surfaces:

- `pnpm --filter app dev` (Vite dev server)
- `pnpm --filter app build` (static bundle under `app/dist/`)
- `cargo tauri dev` / release bundle (Tauri loads `app/dist` as `tauri://localhost/…`)

The copied tree is gitignored (`web/packages/app/public/monaco-host/`) — the source of truth is the monaco-host package. If you change that package, the predev/prebuild hook rebuilds it on the next `dev`/`build`.

The three session-scoped filesystem commands backing this pane — `read_file`, `write_file`, `tree` — look up the workspace root from a server-side cache (`SessionConnections.workspace_root`) populated at `session_hello` time. The webview never supplies a `workspace_root` parameter, so a compromised or buggy webview cannot widen its filesystem sandbox. See `crates/forge-shell/src/ipc.rs` F-122 block for the authority.

## Command palette (F-157)

The app mounts a keyboard-invoked command palette at shell level. Any module inside `packages/app` can expose an action by calling the registry API:

```ts
import { registerCommand } from './commands';

const dispose = registerCommand({
  id: 'unique-id',              // stable; re-registering replaces the entry
  title: 'Human-readable title',
  run: () => { /* invoked on select */ },
});
// dispose() removes the command later (useful in HMR / tests).
```

The palette itself is a single `<CommandPalette />` rendered by `App.tsx`'s shell wrapper. It opens on `Cmd/Ctrl+K` (primary, matches `docs/architecture/window-hierarchy.md` §3.3) and on `Cmd/Ctrl+Shift+P` (alternate). Escape, backdrop click, or re-pressing the shortcut closes it. Fuzzy filtering, arrow-key navigation, and Enter-to-run are provided by the component.

Built-in commands live in `src/commands/registerBuiltins.ts`. Today that file registers only `open-agent-monitor` (the entry F-153 deferred until the palette existed). Add new built-ins there when the target is app-global; register route-scoped commands from their own routes so the registration follows the component lifecycle.

## Further reading

- [Frontend architecture](../../../docs/frontend/architecture.md)
- [Crate architecture — `forge-shell` (Tauri host)](../../../docs/architecture/crate-architecture.md#37-forge-fs-forge-lsp-forge-term-forge-ipc-forge-cli-forge-shell)
- [Window hierarchy](../../../docs/architecture/window-hierarchy.md)
- [IPC contracts](../../../docs/architecture/ipc-contracts.md)
