# forge-agents

Agent definitions plus the runtime foundation. Parses `<workspace_root>/.agents/*.md` and `~/.agents/*.md` into typed `AgentDef`s (YAML frontmatter + prose body), merges user and workspace scopes so workspace agents win on name collisions, and reads the optional workspace-level `AGENTS.md` preamble. On top of that, exposes `Orchestrator` — the runtime side that instantiates `AgentInstance`s, registers them by `AgentInstanceId`, and forwards lifecycle events on a broadcast stream. Sub-agent spawning (F-134), AGENTS.md injection (F-135), sub-agent banners (F-136), background agents (F-137), and the session-side `AgentMonitor` (F-140) compose on this foundation.

## Role in the workspace

- Depends on: `forge-core` (`AgentInstanceId`), `gray_matter`, `serde`, `anyhow`, `thiserror`, `tokio`, `tokio-stream`, `chrono`.
- Depended on by: upcoming session/orchestrator wiring (see `docs/architecture/crate-architecture.md` §3.4).

## Key types / entry points

Parser:

- `AgentDef` — parsed agent: `name`, optional `description`, prompt `body`, `allowed_paths` glob list, `isolation`.
- `Isolation` — `Trusted` (reserved for built-in skills), `Process` (default), `Container` (placeholder).
- `load_workspace_agents(workspace_root)` / `load_user_agents(user_home)` — scan `<root>/.agents/*.md`.
- `load_agents(workspace_root, user_home)` — merged loader: user first, workspace overlays by name.
- `load_agents_md(workspace_root)` — read the optional `AGENTS.md` preamble.
- `AgentLoader::{load, agents, agents_md}` — bundled one-shot loader used per session.

Runtime:

- `Orchestrator::new()` — build a registry + broadcast bus.
- `Orchestrator::spawn(def, ctx)` — instantiate, register, emit `AgentEvent::Spawned`. Rejects user-scope `Isolation::Trusted` with the typed `Error::IsolationViolation` (enforced at parse time *and* again at runtime, covering programmatically-constructed defs).
- `Orchestrator::stop(id)` / `Orchestrator::fail(id, reason)` — terminate an instance, emit the terminal `Completed` / `Failed` event.
- `Orchestrator::record_step_started(id, step)` / `Orchestrator::record_step_finished(id, step)` — emit step transitions; `InstanceState` is not mutated (steps are events, not states).
- `Orchestrator::state_stream()` — `BroadcastStream<AgentEvent>` subscribers consume to follow per-instance progress.
- `AgentInstance { id, def, state, started_at }` — one live instantiation.
- `AgentEvent::{Spawned, StepStarted, StepFinished, Completed, Failed}` — per-instance lifecycle vocabulary.
- `SpawnContext::user()` / `SpawnContext::built_in()` — origin marker that governs the trusted-isolation escape hatch.

## Further reading

- [Crate architecture — `forge-agents`](../../docs/architecture/crate-architecture.md#34-forge-agents)
