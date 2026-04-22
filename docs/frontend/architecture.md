# Frontend Architecture

> Extracted from IMPLEMENTATION.md §9 — Solid signals, state model, Monaco iframe hosting, streaming, and token pipeline

---

## 9. Frontend architecture — Solid

### 9.1 Stack
- **Solid** — fine-grained reactivity, JSX-based. Built for high-frequency updates; streaming chat deltas update only the affected text node.
- **TypeScript** strict, `exactOptionalPropertyTypes` on
- **Vite** for dev and build (with `vite-plugin-solid`)
- **State** — Solid signals directly. No Redux, no Zustand — we don't need them. Global stores are plain modules exporting signals.
- **CSS** — plain CSS variables from DESIGN.md tokens, no Tailwind (Tailwind fights the hand-tuned component system)
- **Routing** — `@solidjs/router` or a minimal custom router (roughly 5 routes: Dashboard, Session, Catalog, Usage, Settings)
- **Monaco** — loaded in an iframe (see §9.3)
- **LSP client** — `monaco-languageclient` runs inside the Monaco iframe but never talks to Tauri directly. It forwards JSON-RPC over `postMessage` to the parent webview, which owns the IPC bridge to `forge-lsp` (see §9.3)
- **Terminal** — xterm.js-compatible rendering driven by byte streams over Tauri events (from `forge-term`)
- **Layout** — persisted `LayoutTree` (from `@forge/ipc`) rendered by `GridContainer`; `SplitPane` handles resize, `useDragToDock` + `DropZoneOverlay` drive drag-to-dock, `usePaneWidth` yields compactness buckets. See §9.2.1 for the full pipeline.

### 9.2 State model

Solid signals at the module level, living under `web/packages/app/src/stores/`. Each store is a plain module exporting signals/stores — no framework wrapping. Six stores cover the product surface; a seventh (`sessionTelemetry`) aggregates per-session provider/cost telemetry derived from wire events.

| Store | File | Role |
|-------|------|------|
| `session` | `web/packages/app/src/stores/session.ts` | Active session id, per-session summary, last event slot, workspace root |
| `approvals` | `web/packages/app/src/stores/approvals.ts` | Tool-call approval whitelist keys + persistence bridge |
| `catalog` | `web/packages/app/src/stores/catalog.ts` | Providers, MCP servers, skills, agents, containers — seeded via `list_*` IPC |
| `messages` | `web/packages/app/src/stores/messages.ts` | Per-session message tree + streaming deltas + tool-call lifecycle |
| `settings` | `web/packages/app/src/stores/settings.ts` | Effective (merged user+workspace) settings; writes flow through `setSetting` IPC |
| `usage` | `web/packages/app/src/stores/usage.ts` | Usage range selector + latest `UsageReport` |
| `sessionTelemetry` | `web/packages/app/src/stores/sessionTelemetry.ts` | Per-session live provider/model + running tokens/cost (F-395) |

```ts
// web/packages/app/src/stores/session.ts
import { createSignal } from 'solid-js';
import { createStore } from 'solid-js/store';
import type { SessionId, SessionState } from '@forge/ipc';

export const [activeSessionId, setActiveSessionId] = createSignal<SessionId | null>(null);
export const [activeWorkspaceRoot, setActiveWorkspaceRoot] = createSignal<string | null>(null);
export const [sessions, setSessions] = createStore<Record<SessionId, SessionSummary>>({});
```

```ts
// web/packages/app/src/stores/catalog.ts
export const [providers, setProviders] = createStore<ProviderSummary[]>([]);
export const [mcpServers, setMcpServers] = createStore<McpServerInfo[]>([]);
export const [skills, setSkills] = createStore<SkillInfo[]>([]);
export const [agents, setAgents] = createStore<AgentInfo[]>([]);
export const [containers, setContainers] = createStore<ContainerSummary[]>([]);
// seeded via `list_*` commands; refreshed on catalog events
```

```ts
// web/packages/app/src/stores/usage.ts
export const [usageRange, setUsageRange] = createSignal<UsageRange>({ kind: 'last_days', days: 7 });
export const [usageReport, setUsageReport] = createSignal<UsageReport | null>(null);
```

Components subscribe by calling the signal as a function (Solid idiom): `{messages[sessionId].map(m => ...)}`. Actions that mutate remote state go through the IPC client in `web/packages/app/src/ipc/`; wire-event handlers call `setSessions` / `setMessages` / `setTelemetry` etc. via `produce` or `reconcile` so fine-grained reactivity survives each delta.

### 9.2.1 Layout model

Pane grid, split resizing, and drag-to-dock live under `web/packages/app/src/layout/`. The model is driven by a persisted `LayoutTree` shape (from `@forge/ipc`) so a tree can round-trip to disk without closure-stripping, and consumed by `GridContainer` which delegates leaf rendering to the caller. See `docs/ui-specs/shell.md` (window shell) and `docs/ui-specs/layout-panes.md` (pane model, drop zones, minimum widths) for the behavioral spec.

| Module | File | Role |
|--------|------|------|
| `GridContainer` | `web/packages/app/src/layout/GridContainer.tsx` | Renders a `LayoutTree` recursively; dispatches leaves through a caller `renderLeaf(leaf)` prop (pane-agnostic) |
| `SplitPane` | `web/packages/app/src/layout/SplitPane.tsx` | Two-child resizable split; 4px snap drag, double-click reset, ARIA window-splitter keyboard step |
| `DropZoneOverlay` | `web/packages/app/src/layout/DropZoneOverlay.tsx` | Presentational top/bottom/left/right/center zones painted during drag (F-118) |
| `layoutStore` | `web/packages/app/src/layout/layoutStore.ts` | Owns the per-workspace `Layouts` record; debounced persistence via `read_layouts` / `write_layouts` IPC (F-120, F-150) |
| `useDragToDock` | `web/packages/app/src/layout/useDragToDock.ts` | Pointer-driven drag hook — emits mutated `LayoutTree` on drop, exposes `DragState` for overlays |
| `usePaneWidth` | `web/packages/app/src/layout/usePaneWidth.ts` | Observes pane content-box width; buckets into `full` / `compact` / `icon-only` per `layout-panes.md §3.7` |
| `dockDrop` | `web/packages/app/src/layout/dockDrop.ts` | Pure tree-mutation kernel — computes the new `LayoutTree` from (source, target, zone) |

Pane bodies (the `renderLeaf` consumers) live under `web/packages/app/src/panes/` — e.g. `EditorPane.tsx`, `TerminalPane.tsx`. Each pane keeps its own effects; the layout layer stays pane-agnostic.

### 9.3 Monaco hosting
- Monaco runs in `<iframe src="/monaco-host.html">` inside the session pane. The iframe is renderer-only; it never talks to Tauri directly
- Communicates with the Solid app via `window.postMessage` using a typed protocol (mirrors IPC types). See [`web/packages/monaco-host/README.md`](../../web/packages/monaco-host/README.md) for the full `kind`-tagged message table
- `monaco-languageclient` runs inside the iframe but the **parent webview owns the LSP bridge**. The iframe emits outbound LSP frames as `client.message` / `client.notification` postMessage envelopes; the parent relays them to `forge-lsp` via the `lsp_send` Tauri command and pipes inbound frames back as `client.message` (bound to the owner webview via the `lsp_message` event — F-062 discipline). Lifecycle is driven from the parent through `lsp_start` / `lsp_stop` in `crates/forge-shell/src/ipc.rs` (F-123; PR #286)
- Reasons: Monaco's AMD loader, web workers, and global scope assumptions don't play well with modern bundlers, and keeping it isolated makes the editor replaceable later. Sandbox-wise, routing LSP through the parent keeps Tauri capabilities off the iframe origin so server output can never touch the filesystem
- **Considered alternatives.** Spawning language servers directly from the iframe (as an earlier draft of this section described) would save one hop of latency, but it requires exposing Tauri process-spawn capability inside the iframe sandbox. That violates the isolation rationale above, so the parent-relay model won

### 9.4 Design token pipeline
- `web/packages/design/src/tokens.css` is the single source of CSS vars
- Checked against DESIGN.md by a CI script (`scripts/check-tokens.sh`) that parses both and fails on drift
- Component files never use raw hex — lint rule enforces `var(--color-*)`

### 9.5 Streaming in the UI

Streaming is where Solid shines. The event handler for `SessionEvent::AssistantDelta` updates the target message's text buffer via `produce` on the `messages` store (`web/packages/app/src/stores/messages.ts`):

```ts
// event handler for AssistantDelta
case 'AssistantDelta':
  setMessages(e.session, e.id, produce(m => {
    m.text += e.delta;
  }));
  break;
```

The `StreamingCursor` component reads `message.streamFinalised` — a signal — and mounts/unmounts reactively. No component re-renders the whole message; only the text span that derives from the specific field updates.
