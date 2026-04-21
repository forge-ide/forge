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

## Tests

Most of the suite runs under the default `cargo test -p forge-mcp` lane.
One integration test is gated off that lane:

- `tests/manager_subprocess.rs` — end-to-end `McpManager` + stdio
  transport + real subprocess composition (Healthy → forced crash →
  Degraded → restart via the backoff ladder → tool-call round-trip).
  Marked `#[ignore]` because tokio installs a single process-wide
  `SIGCHLD` reaper per process: when `cargo test` runs multiple test
  binaries in parallel and more than one of them spawns children, their
  exits race the reaper that a previous binary installed and this test
  observes spurious transport failures. The manager is not at fault —
  it's a test-harness interaction, not a correctness bug.

  CI runs it in a dedicated **single-binary, single-thread** pass via
  `just test-rust-serial` (`cargo test -p forge-mcp -- --ignored
  --test-threads=1`). Locally: `just test-rust-serial`.

  **Do not lift the `#[ignore]` attribute** to make this test run under
  the default parallel suite — the race is reproducible and the serial
  gate is deliberate.

## Further reading

- [Crate architecture — `forge-mcp`](../../docs/architecture/crate-architecture.md#33-forge-mcp)
