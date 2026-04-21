# forge-session

The session daemon — `forged` — and its supporting library. A long-running per-session process that owns the append-only event log, hosts the agent orchestrator and tool dispatcher, accepts client connections over a Unix domain socket under `$XDG_RUNTIME_DIR/forge/sessions/<id>.sock`, and on shutdown either archives or purges the session directory based on the `SessionPersistence` mode. The library half (`forge_session`) exposes the building blocks used by integration tests and the `forge` CLI; the binary half (`forged`) wires them together.

## Role in the workspace

- Depended on by: `forge-cli` (spawns `forged`, talks to its UDS), `forge-shell` (dev-deps for integration tests).
- Depends on: `forge-core`, `forge-fs`, `forge-ipc`, `forge-providers`, `tokio` (full async runtime), `libc` (resource limits and signal plumbing).

## Key types / entry points

- `bin/forged` (`src/main.rs`) — the daemon entrypoint that parses args, builds a `Session`, binds the UDS, and runs to completion.
- `session::Session` — owns the event log, orchestrator, and per-client I/O loops.
- `server` — UDS listener, frame routing, signal-driven shutdown that emits `SessionEnded`.
- `orchestrator` — drives provider chat turns and dispatches tool calls.
- `tools/` — built-in tool implementations (fs, shell, etc.) wired through `forge-fs` for write paths.
- `archive::archive_or_purge` — end-of-life handling: rename into `.forge/sessions/archived/<id>/` or remove the session directory.
- `socket_path` — resolves the session UDS path with the `XDG_RUNTIME_DIR` / `FORGE_SOCKET_PATH` policy.
- `pid_file` — daemon liveness file with stale-pid detection. Cross-platform: the start-time token uses `/proc/self/stat` on Linux, `libproc`'s `BSDInfo` on macOS, and `GetProcessTimes` on Windows (see `starttime`).
- `starttime` — platform-gated `read_self_starttime()` that produces the opaque `u64` token the pid-file embeds so `forge session kill` can detect pid reuse before signalling.
- `sandbox` — `pre_exec` hooks that cap NPROC/NOFILE/FSIZE for spawned children.
- `byte_budget` — per-session aggregate byte budget used to bound output growth.
- `provider_spec` — parses the `provider:model` selector strings the CLI accepts.
- `error::SessionError` — the session-local error type.

## Platform notes

- **macOS `forged`** runs the full persistent-mode lifecycle (F-338), including the pid-file write-and-remove cycle. The `session_kill` path, however, is still Linux-only: race-free signal delivery requires `pidfd_open` / `pidfd_send_signal`, which has no direct macOS equivalent (`kqueue EVFILT_PROC` is the planned follow-up). On macOS, terminate a persistent daemon manually via Activity Monitor, `kill`, or equivalent. `forge session kill` returns a typed error rather than falling back to `libc::kill`, which would reintroduce the pid-reuse race that F-049 closed.
- **Windows** is not a supported host for `forged` today; the UDS handshake and signal plumbing are Unix-only. Windows-specific starttime code exists so the `forge-session` library compiles on Windows for future cross-compilation work.

## Further reading

- [Crate architecture — `forge-session`](../../docs/architecture/crate-architecture.md#35-forge-session)
- [Session UDS protocol (ADR-001)](../../docs/architecture/ADR-001-session-uds-protocol.md)
- [Session layout on disk](../../docs/architecture/session-layout.md)
- [IPC contracts](../../docs/architecture/ipc-contracts.md)
