# Isolation Model

> Extracted from CONCEPT.md §6 and IMPLEMENTATION.md §8 — the three isolation levels, approval model, and sandboxing implementation

---

## 6. Sandboxing model

Agents and MCP servers are untrusted code running with access to the user's files and network. The sandbox story has to be real.

### 6.1 Three levels of isolation

| Level | Mechanism | Who uses it |
|---|---|---|
| **0 — Trusted** | None. Runs in session process. | **Built-in skills only.** User-defined agents cannot declare this. |
| **1 — Process** | Separate OS process, restricted env, fs-scope per `allowed_paths`. | **Default for user-defined agents and MCP servers.** |
| **2 — Container** | OCI (podman preferred, docker fallback). Per-session rootfs, network policy, resource caps. | Opt-in for risky agents or CI-style runs. |

User-defined agents that omit `isolation:` get Level 1 automatically. Level 0 is reserved for code Forge ships.

### 6.2 Frontmatter declaration

```yaml
---
name: refactor-bot
provider: anthropic
model: sonnet-4.5
isolation: process              # or: container (trusted is built-in only)
allowed_tools: [fs.read, fs.write, shell.exec]
allowed_paths: ["./src", "./tests"]
allowed_mcp: [github]
max_tokens: 8000
---
```

Prose body (after frontmatter) is the system prompt.

### 6.3 Approval and isolation are orthogonal

Sandbox enforces **runtime containment**. Approval enforces **human-in-the-loop**. Both apply independently.

| Tool category | Level 0 | Level 1 | Level 2 |
|---|---|---|---|
| Read | auto-approved | auto-approved | auto-approved |
| Write | approval required | approval required | approval required |
| Execute | not allowed | approval required | approval required |
| Network | not allowed | open (no approval per call) | `allowed_hosts` only, no approval |

A containerized agent still needs approval for writes. A trusted built-in skill doing a read doesn't need approval. The two systems do different jobs.

### 6.4 Level 1 networking is open

Process-isolated agents can reach the network freely. Forge does not firewall at the process level. MCP servers and built-in tools like `fetch` do their own allow-listing. This is a deliberate tradeoff — Level 1 is a filesystem and privilege sandbox, not a network sandbox. Users who need network restriction choose Level 2.

### 6.5 Sub-agents use independent isolation

A spawned sub-agent uses its own declared isolation level, not the parent's. Since Level 0 is built-in-only, this means user-defined sub-agents can only be Level 1 or Level 2 — no escape hatch exists for user code to gain trusted status via spawn.

### 6.6 Approval granularity

Approval scope is chosen at the prompt. The user picks:
- **Once** — approve this exact call only; next one prompts again
- **This file** — approve this tool for this specific file/path for the session
- **This pattern** — approve this tool for the matching glob (e.g. `./src/*`) for the session
- **This tool** — approve the tool type entirely for the session (e.g. all `fs.write`)

Whitelist scope is **session only** — never persisted across sessions. At session end, all approvals reset. Keyboard: `R` reject, `A` approve once, `F` approve file, `P` approve pattern, `T` approve tool.

### 6.7 Container management

Forge ships an OCI manager using `oci-spec-rs` and shelling to `podman` or `docker`. v1 requires the user to have podman or docker installed; bundling a runtime is deferred. Dashboard onboarding detects missing runtimes and surfaces install instructions. Images pulled on first use, layers cached.

---

## 8. Sandboxing implementation

### 8.1 Level 0 — Trusted
Tool calls run in the session process. **Only built-in skills** (code Forge ships, never user-authored agents). No subprocess invocation. Enforced at agent parse time: any `isolation: trusted` in a user-authored `.agents/*.md` is rejected.

### 8.2 Level 1 — Process (default for user agents + MCP servers)

Implementation:
- `tokio::process::Command`
- `clearenv`; re-inject whitelisted env vars only (`PATH`, `HOME`, `LANG`, `LC_*`, session-specific `FORGE_SESSION_ID`)
- Path access enforced by `forge-fs`: every `fs.*` tool validates the path against the agent's `allowed_paths` glob
- **Network is open at Level 1.** No per-agent firewall. MCP servers and the built-in `fetch` tool do their own allow-listing. Users who need network restriction use Level 2.
- CPU/RAM: soft limits via `setrlimit` (Linux/macOS)
- **Per-sandbox process ceiling via cgroup v2 `pids.max` (F-149).** Each sandbox gets its own leaf under the daemon's cgroup parent so a misbehaving tool cannot starve sibling sandboxes or the daemon itself. Linux-only; requires the host to delegate the `pids` controller to the daemon's slice (default on systemd user sessions). On non-delegated hosts (cgroup v1, containers without delegation, non-Linux) setup is skipped silently and `RLIMIT_NPROC` becomes the only ceiling. `RLIMIT_NPROC` is retained as a uid-wide backstop regardless. See [`docs/dev/sandbox-limits.md`](../dev/sandbox-limits.md) for the full operator-facing reference.
- Kill on session end: process group guarantees cleanup

### 8.3 Level 2 — Container
Spec generated at session/agent start and handed to podman/docker.

Image strategy:
- Base images maintained by us: `oci.io/forge/rust-tools:<ver>`, `oci.io/forge/node-tools:<ver>`, `oci.io/forge/py-tools:<ver>`
- User may specify their own in `.agents/<name>.md`:
  ```yaml
  isolation:
    kind: container
    image: docker.io/library/python:3.12
  ```

Mounts:
- Workspace mounted at `/workspace` read-write by default, read-only if declared
- `~/.config/forge/certs/` mounted at `/etc/forge/certs/` for provider access
- No home dir, no `/tmp` cross-mount

Network:
- Default: no network
- Declared hosts (for MCP or tools): CNI policy allows only those

Resource limits:
- Enforced by runtime via OCI spec (`linux.resources.memory`, `cpu.shares`)

### 8.4 Approval — orthogonal to isolation

Sandbox enforces runtime containment. Approval enforces human-in-the-loop. They operate independently. Writes, exec, and network-side-effect tools require approval regardless of isolation level, per the matrix in §6.3.

Approval granularity comes in four scopes (once/file/pattern/tool) — see SPECS.md §10. Whitelists are session-local; no persistent whitelists.
