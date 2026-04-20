# forge-mcp

MCP (Model Context Protocol) client and server lifecycle manager. Phase 2 (F-127) landed the config-parsing foundation: the universal `.mcp.json` schema parser and typed `McpServerSpec` / `ServerKind` values. Transports (stdio F-128, http F-129) and the lifecycle manager (F-130) build on top.

## Role in the workspace

- Depended on by: nothing yet; will be consumed by `forge-session`'s tool dispatcher when MCP support ships.
- Depends on: `serde`, `serde_json`, `anyhow`, `dirs`.

## Key types / entry points

- `McpServerSpec { kind: ServerKind }` — parsed declaration of a single server.
- `ServerKind::Stdio { command, args, env }` / `ServerKind::Http { url, headers }` — transport-specific shapes.
- `config::load_workspace(root)` — read `<root>/.mcp.json`.
- `config::load_user()` — read `~/.config/forge/mcp.json` (XDG-resolved).
- `config::load_user_from(config_dir)` — test-friendly variant that takes an explicit base directory.
- `config::load_merged(workspace_root, user_config_dir)` — merge both scopes; workspace wins on name collision.

The `mcpServers` schema is the universal proposal (MCP repo discussion #2218). Transport is discriminated by an explicit `"type"` field (`"stdio"` / `"http"`) when present, otherwise inferred from the presence of `command` (stdio) or `url` (http). Unknown fields are rejected.

The remaining planned API (`McpManager`, `Scope`, `ImportSource`, `McpStateEvent`, the stdio / HTTP transports) lands in F-128 / F-129 / F-130.

## Further reading

- [Crate architecture — `forge-mcp`](../../docs/architecture/crate-architecture.md#33-forge-mcp)
