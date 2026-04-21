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

### 9.2 State model

Solid signals at the module level. Three main stores:

```ts
// web/packages/state/src/session.ts
import { createSignal, createMemo } from 'solid-js';
import { createStore, produce } from 'solid-js/store';

export const [activeSessionId, setActiveSessionId] = createSignal<SessionId | null>(null);
export const [sessions, setSessions] = createStore<Record<SessionId, SessionState>>({});

export const activeSession = createMemo(() => {
  const id = activeSessionId();
  return id ? sessions[id] : null;
});

// actions (each goes through IPC client)
export async function sendMessage(session: SessionId, text: string, ctx: ContextRef[]) {
  await ipc.sendMessage(session, text, ctx);
  // event-driven update via `forge://event` subscription
}

export async function approveTool(session: SessionId, id: ToolCallId, scope: ApprovalScope) {
  await ipc.approveToolCall(session, id, scope);
}
```

```ts
// web/packages/state/src/catalog.ts
export const [providers, setProviders] = createStore<ProviderSummary[]>([]);
export const [mcpServers, setMcpServers] = createStore<McpServerInfo[]>([]);
export const [skills, setSkills] = createStore<SkillInfo[]>([]);
export const [agents, setAgents] = createStore<AgentInfo[]>([]);
export const [containers, setContainers] = createStore<ContainerSummary[]>([]);
// refreshed on events; bootstrapped via `list_*` commands
```

```ts
// web/packages/state/src/usage.ts
export const [usageRange, setUsageRange] = createSignal<UsageRange>({ kind: 'last_days', days: 7 });
export const [usageReport, setUsageReport] = createSignal<UsageReport | null>(null);
```

Components subscribe by calling the signal as a function (Solid idiom): `{sessions[id].messages.map(m => ...)}`.

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

Streaming is where Solid shines. The event handler for `SessionEvent::AssistantDelta` updates the target message's text buffer via `produce`:

```ts
// event handler for AssistantDelta
case 'AssistantDelta':
  setSessions(e.session, 'messages', e.id, produce(m => {
    m.text += e.delta;
  }));
  break;
```

The `StreamingCursor` component reads `message.streamFinalised` — a signal — and mounts/unmounts reactively. No component re-renders the whole message; only the text span that derives from the specific field updates.
