# forge-mcp

MCP (Model Context Protocol) client and server lifecycle manager. Phase 2 (F-127) landed the config-parsing foundation; F-128 adds the stdio transport. HTTP (F-129) and the lifecycle manager (F-130) build on top.

## Role in the workspace

- Depended on by: `forge-session`, `forge-shell`, and `forge-ipc` (which re-exports [`McpServerInfo`] for IPC wire types).
- Depends on: `forge-core` (re-exported `McpStateEvent` / `ServerState`), `serde`, `serde_json`, `anyhow`, `dirs`, `tokio`, `tracing`.

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

The lifecycle manager landed in F-130 and has been hardened through F-155:

- `McpManager` — owns per-server state, restart budgets, and the tool-call path.
- `McpServerInfo` — snapshot shape consumed by `forge-ipc` and the webview.
- `LifecycleTuning`, `HEALTH_CHECK_INTERVAL`, `MAX_RESTARTS_PER_WINDOW`, `REQUEST_TIMEOUT`, `RESTART_BACKOFF_LADDER`, `RESTART_WINDOW` — tunables for the restart ladder and health loop.
- `McpStateEvent` / `ServerState` — re-exported from `forge-core` so existing callers keep resolving at `forge_mcp::*`; the types live upstream to break a dependency cycle with `forge-core::Event`.

## Security: stdio child environment is deny-by-default

`transport::Stdio::connect` clears the inherited process environment before spawning an MCP server and re-injects only an explicit allow-list, with the spec's `env` map layered on top. MCP servers declared in `.mcp.json` **never** see parent-process secrets (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GITHUB_TOKEN`, `AWS_*`, arbitrary shell exports) unless the spec re-declares them explicitly.

Rationale: `.mcp.json` is a checked-in workspace file; any collaborator or merged workspace can add a server entry. Default-inherit would make it a silent exfiltration channel (F-345, CWE-526 / CWE-200).

Forwarded by default (read from the parent if present, skipped otherwise):

`PATH`, `HOME`, `LANG`, `LC_ALL`, `USER`, `LOGNAME`, `TMPDIR`, `TMP`, `TEMP`, `SystemRoot`, `ComSpec`, `PATHEXT`.

Callers that need the child to see additional parent-held values must surface them through the spec's `env` map — the workspace config — not the daemon's ambient environment.

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
