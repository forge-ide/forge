# forge-mcp

MCP (Model Context Protocol) client and server lifecycle manager. Reserved scaffold in Phase 1 — the crate compiles into the workspace so dependent crates can be wired against it as MCP support lands, but the implementation work (loading `.mcp.json` / `~/.mcp.json`, spawning stdio and HTTP servers, restart policy, importing from other tool ecosystems, exposing tools through the provider-agnostic `Tool` shape) is deferred to a later milestone per the architecture doc.

## Role in the workspace

- Depended on by: nothing yet; will be consumed by `forge-session`'s tool dispatcher when MCP support ships.
- Depends on: nothing (intentionally empty until implementation begins).

## Key types / entry points

- _None yet._ The planned API (`McpManager`, `Scope`, `ImportSource`, `McpStateEvent`, the `mcpServers` JSON parsers, and the stdio / HTTP transports) is documented in the architecture doc and will land with the implementation.

## Further reading

- [Crate architecture — `forge-mcp`](../../docs/architecture/crate-architecture.md#33-forge-mcp)
