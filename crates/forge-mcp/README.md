# forge-mcp

MCP (Model Context Protocol) client and server lifecycle manager. Phase 2 (F-127) landed the config-parsing foundation; F-128 adds the stdio transport. HTTP (F-129) and the lifecycle manager (F-130) build on top.

## Role in the workspace

- Depended on by: nothing yet; will be consumed by `forge-session`'s tool dispatcher when MCP support ships.
- Depends on: `serde`, `serde_json`, `anyhow`, `dirs`, `tokio`, `tracing`.

## Key types / entry points

- `McpServerSpec { kind: ServerKind }` — parsed declaration of a single server.
- `ServerKind::Stdio { command, args, env }` / `ServerKind::Http { url, headers }` — transport-specific shapes.
- `config::load_workspace(root)` — read `<root>/.mcp.json`.
- `config::load_user()` — read `~/.mcp.json` from the user's home directory.
- `config::load_user_from(home_dir)` — test-friendly variant that takes an explicit home directory.
- `config::load_merged(workspace_root, user_config_dir)` — merge both scopes; workspace wins on name collision.
- `transport::Stdio::connect(spec)` — spawn a stdio MCP subprocess and open a JSON-RPC channel.
- `transport::Stdio::send(value)` / `transport::Stdio::recv()` — line-delimited JSON-RPC round-trip; `recv` yields `StdioEvent::Message(..)` frames and, once the child exits, exactly one terminal `StdioEvent::Exit(..)`.

The `mcpServers` schema is the universal proposal (MCP repo discussion #2218). Transport is discriminated by an explicit `"type"` field (`"stdio"` / `"http"`) when present, otherwise inferred from the presence of `command` (stdio) or `url` (http). Unknown fields are rejected.

The remaining planned API (`McpManager`, `Scope`, `ImportSource`, `McpStateEvent`, HTTP transport) lands in F-129 / F-130.

## Further reading

- [Crate architecture — `forge-mcp`](../../docs/architecture/crate-architecture.md#33-forge-mcp)
