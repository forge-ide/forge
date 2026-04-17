# AI UX

> Extracted from CONCEPT.md — transparency, @-context, multi-provider, memory, and all AI-specific UX commitments

---

## 10. AI-specific UX commitments

These patterns define Forge's AI surface.

### 10.1 Transparent by default
- Every tool call renders inline in chat as a collapsible card (per SPECS.md).
- Every sub-agent spawn is banner-rendered in the parent thread.
- Token usage and cost are shown live in the pane header — never hidden in settings.
- Streaming state is the ember blink cursor; never a spinner.

### 10.2 `@`-context is universal
Any chat input accepts `@` to insert a reference: `@file.rs`, `@dir/`, `@selection`, `@terminal-output`, `@agent:refactor-bot`. The same syntax works across all providers via per-provider adaptation at send time (XML blocks for Anthropic, function-result blocks for OpenAI, etc.).

### 10.3 Multi-provider per session
A single chat thread can shift providers mid-conversation. The provider accent color on each message bubble marks origin.

**Re-run a response** via the message's re-run action, which opens a three-option menu:
- **Replace** — re-run keeping the conversation up to (but not including) this message. Original gone, new one takes its place.
- **Branch** — re-run keeping both. Original stays; new answer appears as a sibling branch, user can switch between. See SPECS.md for the branching UI.
- **Fresh** — re-run with only the original user message. Loses intermediate tool calls and sub-agent context.

### 10.4 Context window management
- **At 90% of the provider's context window:** pane header shows an approaching-limit warning
- **At 98%:** Forge automatically compacts oldest turns into a summary message. User is notified via toast; compacted message shows inline with a "compacted N turns" marker. Compaction uses the session's active provider (cost counted toward session usage).
- **Any time:** `Compact now` action available in the palette and as a pane header button. Lets the user proactively trim before a long tool call run.

Compaction affects only what the model sees next turn. The event log remains fully intact.

### 10.5 Memory — off by default, opt-in
Forge has no cross-session memory by default. Each session is independent.

With the setting `memory.cross_session.enabled = true`, agents gain a per-agent memory store at `~/.config/forge/memory/<agent-name>.md`. The file's contents are auto-injected into the agent's context; the agent can call `memory.append` / `memory.update` / `memory.read` to modify it. Memory is per-agent, not global — `refactor-bot`'s memory isn't shared with `doc-writer`.

This is off by default because cross-session memory has real privacy and predictability implications. Users who want it opt in explicitly.

### 10.6 Background agents
Two patterns for asynchronous work, both first-class:
- **Sub-agents** — spawned by an agent as part of orchestration. Appear inline as sub-agent banners in the parent's chat thread. See SPECS.md.
- **Background agents** — user-initiated. User starts an agent that runs asynchronously alongside the current chat. Lives in the Agent Monitor, not the main chat. Status bar shows a small `N bg` indicator when background agents are running. On completion, notifies per the `notifications.bg_agents` setting (`toast` | `os` | `both` | `silent`; default `toast`).

Background agents can be promoted to a main chat pane if the user wants to continue interactively. `forge run agent <name>` is the CLI equivalent of a background agent.

### 10.7 Parallel tool calls — reads only
When a provider issues multiple tool calls in one turn:
- **Read-only tools** (`fs.read`, `fs.list`, declared-readonly MCP calls) execute in parallel
- **Writes, exec, network-side-effect tools** execute sequentially, each with its own approval prompt

For mixed batches, Forge runs reads in parallel first, then serializes the writes. The tool-call UI shows parallel reads with a small `[N parallel]` indicator.

Classification comes from tool metadata: built-in tools declare it; MCP tools advertise it via the MCP `readOnly` hint. Default is write-classification (safe) for anything that doesn't explicitly claim read-only.

### 10.8 Human-in-the-loop by default
Tool calls that write, execute, or reach network outside declared hosts require approval (per §6.3). The approval UI is inline, not modal (per SPECS.md §10). Approval scope is chosen at the prompt (per §6.6). Approval state is session-local.

### 10.9 Interruption is first-class
Stop button always available during streaming. `Esc` works everywhere. A stopped response is kept, labeled `interrupted`, with a one-click resume. An interrupted tool call is rolled back where possible — writes are staged behind approval anyway, so nothing is half-applied.
