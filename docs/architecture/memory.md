# Cross-session memory

Status: stable as of F-601 (Phase 3).

Forge agents can opt into a small per-agent Markdown file that persists
across sessions. When enabled, the file's body is appended to the agent's
system prompt under a `## Memory` heading after `AGENTS.md`, and a
`memory.write` tool is exposed so the agent can update its own memory.

This document is the contract for `forge_agents::memory` and the wiring in
`forge_session::server::serve_with_session`. See also
`docs/architecture/crate-architecture.md` §3.4.

## Scope

- One Markdown file per agent. Multiple sessions of the same agent share
  the same memory.
- Memory is a coarse, agent-scoped scratchpad — not a key-value store, not
  a vector store, not session history.
- Reads are best-effort: a corrupt file logs a warning and the session
  continues without memory injection.
- Writes are atomic (temp file + rename) and increment a monotonic version
  on every write.

## Storage layout

The store is rooted at the platform's user-config directory, resolved via
`dirs::config_dir`:

| Platform | Path |
|----------|------|
| Linux    | `$XDG_CONFIG_HOME/forge/memory/<agent>.md` (default `~/.config/forge/memory/<agent>.md`) |
| macOS    | `~/Library/Application Support/forge/memory/<agent>.md` |
| Windows  | `%APPDATA%\forge\memory\<agent>.md` |

`<agent>` is the agent name from the `.agents/<name>.md` frontmatter (or
the filename stem when frontmatter omits it). The same string is used to
locate the memory file and to seed the `memory.write` tool registration.

The parent directory is created on first write at mode `0700` on Unix; on
Windows the platform default ACL applies.

## File format

```markdown
---
updated_at: 2026-04-26T12:00:00Z
version: 1
---
free-form markdown body the agent has accumulated
```

Frontmatter fields:

| Field        | Type    | Required | Notes                               |
|--------------|---------|----------|-------------------------------------|
| `updated_at` | ISO 8601 / RFC 3339 timestamp | yes | Snaps to `Utc::now()` on every write. |
| `version`    | positive integer | yes | Increments by 1 on every write; first write writes `1`. |

The body is free-form Markdown. Forge does not interpret it — bytes
round-trip verbatim, modulo a single trailing newline that the YAML
frontmatter parser may strip.

`memory.write { mode: "append" }` joins the new content to the existing
body with a single `\n` separator. `mode: "replace"` discards the existing
body in full.

## Per-agent opt-in

Memory is OFF by default. An agent opts in by setting `memory: true`
(or the explicit `memory_enabled: true`) in its frontmatter:

```markdown
---
name: scribe
memory: true
---

You are a long-running scribe...
```

The flag controls **both** behaviors:

1. Whether the agent's memory body is loaded and appended to the system
   prompt.
2. Whether the `memory.write` tool is registered on the dispatcher and
   thus discoverable by the agent.

An agent that has not opted in cannot see the tool name and cannot read
or write memory.

## System-prompt injection

When memory is enabled and the file exists, the assembled system prompt is:

```text
\n\n---\nAGENTS.md (workspace):\n<agents.md content>

---
## Memory
<memory body>
```

Both halves are optional and assembled by `forge_agents::assemble_system_prompt`:

| `AGENTS.md` | Memory body | Result                                 |
|-------------|-------------|----------------------------------------|
| absent      | absent      | `None` — no system prompt              |
| present     | absent      | AGENTS.md prefix only                  |
| absent      | present     | `## Memory` heading + body             |
| present     | present     | AGENTS.md prefix, then `## Memory`     |

The session does the assembly **once** per session start and stores the
result as `Arc<str>` so per-turn cost stays at the existing refcount bump.

## Active-agent selection

`serve_with_session` accepts an `active_agent: Option<String>` parameter
that names which agent's memory backs the session. The Tauri shell knows
which agent each window targets and passes it in explicitly; the daemon
binary (`forged`) reads `FORGE_ACTIVE_AGENT` once at process start and
forwards it as the typed parameter. **The server itself does not read the
environment for this concern** — that distinction matters in
persistent-mode operation, where one daemon serves multiple connections
and a process-global mutable env var would let one window silently see
another agent's memory.

The named agent is looked up in the loaded `AgentDef` set:

- `None` / empty / whitespace → memory off.
- Names an agent that is not loaded → memory off, logged at WARN.
- Names a loaded agent with `memory_enabled: false` → memory off.
- Names a loaded agent with `memory_enabled: true` → memory on; body is
  appended and `memory.write` registers.

## `memory.write` tool

```json
{ "name": "memory.write",
  "args": { "content": "...", "mode": "append" | "replace" } }
```

Returns:

```json
{ "ok": true, "version": 7, "updated_at": "2026-04-26T12:00:00Z" }
```

On failure (missing arg, unknown mode, IO error) the tool returns
`{ "error": "<message>" }` rather than propagating an exception — the
dispatcher contract is that `invoke` always succeeds and the model
decides what to do with the JSON result.

The tool is gated on the per-agent flag: it is registered on the
dispatcher only when the active agent has `memory_enabled: true`. An
agent that has not opted in cannot discover the wire name.

## Security model

**Memory is plain Markdown.** No template evaluation, no executable
content, no macro expansion. Bytes round-trip verbatim through the YAML
frontmatter parser.

**File permissions:**
- `~/.config/forge/memory/` directory: mode `0700` on Unix.
- Each `<agent>.md` file: mode `0600` on Unix.
- Windows: platform default ACL — no encryption, no extra hardening.

**Atomicity:** writes go to `<agent>.md.tmp`, then `rename(2)` to the
final path. A crashing write leaves the prior file intact.

**Best-effort reads:** a corrupt YAML frontmatter, a missing
`updated_at` or `version`, or a `version: 0` value all log a warning and
yield "no memory" — the session never crashes on a malformed file.

### DO NOT store secrets in memory

The memory body is appended verbatim to the system prompt of every
subsequent agent turn. That means anything in memory is:

- visible to the model at every turn,
- transmitted over every provider request (HTTPS or local socket),
- logged or serialized by anything that captures the system prompt.

There is **no encryption, no redaction, no scoping** of the memory body.
If an agent or a user writes a secret (API key, password, private
identifier, customer data) into memory, that secret is now part of the
permanent system prompt for that agent.

Forge does not scan memory for secrets and does not warn at write time —
the `memory.write` tool accepts any string.

**Rule of thumb:** treat the memory file the way you treat
`~/.bash_history` — visible to anyone with shell access, persistent
across reboots, and explicitly **not** a place for credentials.

## Testing

`crates/forge-agents/src/memory.rs` includes unit tests for:

- frontmatter round-trip (write + load),
- `append` vs `replace` semantics,
- monotonic version increment,
- `updated_at` advancement,
- corrupt-frontmatter recovery (returns `None`, no panic),
- file-mode `0600` on Unix.

`crates/forge-session/tests/memory_injection.rs` exercises the
session-level seam: `MemoryStore` + `assemble_system_prompt` composed the
way `serve_with_session` composes them.

`crates/forge-session/src/dispatcher_cache.rs` tests assert that
`memory.write` is registered if-and-only-if a `MemoryWriteBinding` is
provided.
