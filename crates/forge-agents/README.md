# forge-agents

Loads agent definitions from the workspace and user home — `<workspace_root>/.agents/*.md` and `~/.agents/*.md` — parsing each Markdown file's YAML frontmatter into a typed `AgentDef` and merging the two scopes so workspace agents win on name collisions. Also reads the optional workspace-level `AGENTS.md` preamble that callers inject into every agent's system prompt. The richer orchestration responsibilities described in the architecture doc (sub-agent spawning, isolation enforcement, background agents) land in later phases; this Phase 1 crate is the loader.

## Role in the workspace

- Depended on by: future session/orchestrator wiring (see `docs/architecture/crate-architecture.md` §3.4 for the Phase 2+ shape).
- Depends on: `gray_matter` (YAML frontmatter), `serde`, `anyhow`.

## Key types / entry points

- `AgentDef` — parsed agent: `name`, optional `description`, prompt `body`, and `allowed_paths` glob list.
- `parse_agent_file` (private) — single-file parser; rejects `isolation: trusted` for user-defined agents with a clear error.
- `load_workspace_agents(workspace_root)` — load `<root>/.agents/*.md`, returning `Ok(vec![])` if the directory is absent.
- `load_user_agents(user_home)` — load `<home>/.agents/*.md` similarly.
- `load_agents(workspace_root, user_home)` — merged loader: user first, workspace overlays by name.
- `load_agents_md(workspace_root)` — read the optional `AGENTS.md` preamble.
- `AgentLoader::{load, agents, agents_md}` — bundled one-shot loader used per session.

## Further reading

- [Crate architecture — `forge-agents`](../../docs/architecture/crate-architecture.md#34-forge-agents)
