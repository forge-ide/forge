# forge-cli

The `forge` command-line binary. A thin `clap`-derived front end that spawns and talks to `forged` session daemons over their Unix domain sockets — listing sessions, tailing the live event stream, sending SIGTERM, and starting new agent or provider sessions. All session-id arguments are validated at parse time against the canonical `^[0-9a-f]{16}$` shape so attacker-controlled values can never reach the socket-path resolver.

## Role in the workspace

- Depended on by: nothing internal (leaf binary). `forge-session` pulls it in as a dev-dependency for integration tests.
- Depends on: `forge-core`, `forge-ipc`, `clap`, `tokio`, `libc`.

## Key types / entry points

- `bin/forge` (`src/main.rs`) — CLI entrypoint.
- `Cli` / `Commands` — top-level `clap` parser (`session`, `run`).
- `SessionCommands` — `new`, `list`, `tail <id>`, `kill <id>`, with id-format validation via `parse_session_id`.
- `SessionNewKind::{Agent, Provider}` — the two ways to start a new session.
- `RunCommands::Agent` — one-shot ephemeral agent run reading input from `-` (stdin) or a file.
- `socket` — UDS path / PID file resolution and the `session_id_is_valid` gate.
- `ipc` — frame-level helpers used by `tail` and the daemon-talking subcommands.
- `display` — terminal output formatting for events and session listings.

## Further reading

- [Crate architecture — `forge-cli`](../../docs/architecture/crate-architecture.md#37-forge-fs-forge-lsp-forge-term-forge-ipc-forge-cli-forge-shell)
- [IPC contracts](../../docs/architecture/ipc-contracts.md)
