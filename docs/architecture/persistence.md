# Persistence

> Extracted from IMPLEMENTATION.md §7 — filesystem-only persistence, full file layout, metadata schemas, usage aggregation, archive/reactivation

---

## 7. Persistence layer — filesystem only

Forge has no embedded database. All persistent state is on disk as plain files.

### 7.1 Filesystem layout

```
~/.config/forge/
  config.toml              # user-global settings (TOML)
  workspaces.toml          # known-workspaces registry
  credentials.toml         # {provider_id → keychain_ref}
  memory/
    <agent-name>.md        # opt-in cross-session memory

~/.agents/<name>.md        # user-global agent definitions
~/.skills/<name>/          # user-global skills (agentskills.io folders)
  SKILL.md
  scripts/
  references/
~/.mcp.json                # user-global MCP servers

<workspace>/
  AGENTS.md                # shared workspace instructions
  .mcp.json                # workspace MCP servers
  .agents/<name>.md        # workspace agent definitions
  .skills/<name>/SKILL.md  # workspace skills
  .forge/                  # internal, self-gitignored
    .gitignore             # contents: *
    sessions/
      <id>/
        meta.toml
        events.jsonl
        snapshots/
      archived/<id>/       # same layout as above, moved on archive
    layouts.json
    languages.toml         # user-added LSP servers
    cache/                 # fetch results, etc.
```

### 7.2 `workspaces.toml` (user-global registry)

```toml
[[workspace]]
id = "sha1-hex"
path = "/home/alice/code/acme-api"
name = "acme-api"
last_opened = 2026-04-15T14:22:00Z
pinned = false

[[workspace]]
id = "sha1-hex2"
path = "/home/alice/code/docs-v2"
...
```

### 7.3 `meta.toml` (per session)

```toml
id = "a3f1b2c4"
workspace_id = "sha1-hex"
name = "refactor-payment-service"
agent = "refactor-bot"                 # optional; omitted for bare-provider sessions
provider_id = "anthropic"               # for bare-provider sessions
model = "sonnet-4.5"                    # for bare-provider sessions
state = "active"                        # active | archived | ended
persistence = "persist"                 # persist | ephemeral
started_at = 2026-04-15T14:22:00Z
ended_at = ... (optional)
tokens_in = 48200
tokens_out = 12100
cost_usd = 0.23
pid = 48211
socket_path = "/tmp/forge-1000/sessions/a3f1b2c4.sock"
```

### 7.4 Usage aggregation

Each session's event log has `UsageTick` events — the authoritative source. Queries for "spend across all workspaces last 7 days" work by:

1. Scanning known workspaces (from `workspaces.toml`)
2. Reading each session's `events.jsonl` (skipping schema header)
3. Filtering `UsageTick` events within the date range
4. Aggregating

For volume performance, phase 3 may introduce an additive monthly aggregate file (`~/.config/forge/usage-<YYYY-MM>.jsonl`) written on session end. Decision deferred until we have real volume signal.

### 7.5 Archive on session end

When a `persist` session ends:
1. Final events written
2. `meta.toml` updated to `state = "archived"`, `ended_at = <now>`
3. The session directory is moved from `.forge/sessions/<id>/` to `.forge/sessions/archived/<id>/`
4. Socket path removed from runtime dir

When an `ephemeral` session ends:
1. Process terminates
2. The session directory is fully removed from disk
3. No trace persists

### 7.6 Reactivating an archived session

`forge session attach <id>` on an archived session:
1. Looks up `meta.toml` in `archived/<id>/`
2. Moves the directory back to `sessions/<id>/`
3. Spawns `forged` with `--reactivate <id>` flag
4. Session replays event log to restore in-memory state
5. Ready for new activity; state becomes `active` again
