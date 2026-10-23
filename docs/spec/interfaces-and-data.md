# Interfaces and data

The contracts every crate and the UI build against. Rust types in `forge-api` are the source of truth; this file is the human-readable version.

## Conventions

- **IDs** are ULIDs with a type prefix: `prj_`, `ses_`, `trn_` (turn), `chk_` (checkpoint), `apr_` (approval), `thr_` (thread), `cmt_` (comment), `hnk_` (hunk). Events use a per-session sequence number instead.
- **Wire format:** JSON with camelCase fields. Rust types use snake_case with `#[serde(rename_all = "camelCase")]`.
- **Times** are RFC 3339 in UTC. **Paths** on the wire are repo-relative with forward slashes; absolute paths never cross the API.
- **Versioning:** `server.hello` returns the API version (semver). Adding a field or method is a minor bump; removing or changing one is major. Clients ignore unknown fields.

**Errors** use JSON-RPC errors with application codes and a structured `data` field:

```ts
type ErrorData = { kind: ErrorKind; detail: string; retryable: boolean };
```

| Code | Kind | Meaning |
| --- | --- | --- |
| -32001 | `NotFound` | ID or path doesn't exist |
| -32002 | `InvalidState` | Not allowed in the session's current state |
| -32003 | `PolicyDenied` | Policy engine denied the action |
| -32004 | `AgentUnavailable` | Agent not installed, not signed in, or crashed |
| -32005 | `SandboxUnavailable` | Requested tier not supported on this machine |
| -32006 | `Conflict` | Stale write (hash mismatch) or git conflict |
| -32007 | `Unauthorized` | Token missing or lacks scope |
| -32008 | `ValidationFailed` | Params or config failed schema validation |

## Daemon API (v0.1)

The full v0.1 method set. Types are shown in TypeScript because that is what the UI consumes; `forge-api` defines the same shapes in Rust.

```ts
// Core types
type Mode = 'ask' | 'accept-edits' | 'auto' | 'full-access';
type SandboxTier = 'light' | 'standard' | 'container' | 'unsandboxed';
type SessionState = 'provisioning' | 'ready' | 'working' | 'waiting'
  | 'paused' | 'review' | 'done' | 'failed';

type Project = { id: string; name: string; root: string; repos: Repo[] };
type Repo = { name: string; path: string; baseBranch: string };
type AgentProfile = { id: string; agent: 'claude-code' | 'codex' | 'gemini' | string;
  installed: boolean; authenticated: boolean; version?: string };
type Session = { id: string; projectId: string; title: string; agentProfile: string;
  state: SessionState; mode: Mode; sandbox: SandboxTier; branch: string;
  createdAt: string; updatedAt: string; usage: { inputTokens: number;
  outputTokens: number; costUsd?: number } };
type Approval = { id: string; sessionId: string; action: Action; reason: string;
  createdAt: string };
type Action =
  | { type: 'edit'; path: string; kind: 'create' | 'modify' | 'delete' }
  | { type: 'exec'; argv: string[]; cwd: string }
  | { type: 'network'; host: string; port: number }
  | { type: 'mcp'; server: string; tool: string; args: unknown }
  | { type: 'git'; op: string };
type Hunk = { id: string; oldStart: number; oldLines: number; newStart: number;
  newLines: number; lines: string[]; turnId?: string };
type FileChange = { path: string; status: 'added' | 'modified' | 'deleted' | 'renamed';
  oldPath?: string; additions: number; deletions: number };
type Changeset = { sessionId: string; base: string; head: string; files: FileChange[] };
type Thread = { id: string; sessionId: string; path: string;
  range: { startLine: number; endLine: number }; checkpointId: string;
  status: 'open' | 'resolved' | 'outdated'; comments: Comment[] };
type Comment = { id: string; author: 'human' | 'agent'; body: string; createdAt: string };
```

| Method | Params | Result |
| --- | --- | --- |
| `server.hello` | `{ client, apiVersion }` | `{ daemonVersion, apiVersion, platform, sandboxTiers }` |
| `project.open` | `{ path }` | `Project` |
| `project.list` | `{}` | `Project[]` |
| `project.get` | `{ projectId }` | `Project` |
| `agent.list` | `{}` | `AgentProfile[]` |
| `session.create` | `{ projectId, agentProfile, title, prompt, mode, sandbox? }` | `Session` |
| `session.prompt` | `{ sessionId, text }` | `{ turnId }` |
| `session.cancel` | `{ sessionId }` | `{}` |
| `session.setMode` | `{ sessionId, mode, sandbox? }` | `Session` |
| `session.rewind` | `{ sessionId, checkpointId }` | `Session` |
| `session.get` | `{ sessionId }` | `Session` |
| `session.list` | `{ projectId, states? }` | `Session[]` |
| `session.close` | `{ sessionId, outcome: 'shipped' \| 'abandoned' }` | `{}` |
| `approval.list` | `{ projectId? }` | `Approval[]` |
| `approval.decide` | `{ approvalId, decision: 'allow' \| 'deny', remember?: 'session' \| 'project', note? }` | `{}` |
| `review.changeset` | `{ sessionId, fromCheckpoint? }` | `Changeset` |
| `review.fileDiff` | `{ sessionId, path, fromCheckpoint? }` | `{ hunks: Hunk[] }` |
| `review.comment` | `{ sessionId, path, range, body }` | `Thread` |
| `review.reply` | `{ threadId, body }` | `Thread` |
| `review.resolve` | `{ threadId }` | `Thread` |
| `review.threads` | `{ sessionId, status? }` | `Thread[]` |
| `review.requestChanges` | `{ sessionId }` | `{ turnId }` |
| `review.revertHunk` | `{ sessionId, path, hunkId }` | `{}` |
| `review.ship` | `{ sessionId, strategy: 'push-branch' \| 'merge-local', branch? }` | `{ ref }` |
| `file.read` | `{ projectId, path, sessionId? }` | `{ content, hash }` |
| `file.write` | `{ projectId, path, content, baseHash, sessionId? }` | `{ hash }` |
| `lsp.request` | `{ projectId, sessionId?, method, params }` | LSP result |
| `events.subscribe` | `{ topics, afterSeq? }` | `{ subscriptionId }` |
| `events.unsubscribe` | `{ subscriptionId }` | `{}` |
| `events.query` | `{ sessionId, kinds?, text?, before?, limit }` | `Event[]` |

Notifications arrive as `event` with `{ subscriptionId, topic, seq, event }`. Topics: `board`, `approvals`, `session/<id>`, `project/<id>`.

## Events

Every event has the envelope `{ seq, sessionId, at, kind, parent?, body }`. `parent` links a decision, hook run, or result to the tool call it belongs to.

| Kind | Body | Topic |
| --- | --- | --- |
| `session.state` | `{ from, to, reason? }` | `session/<id>`, `board` |
| `turn.started` | `{ turnId, source: 'human' \| 'review' \| 'hook' }` | `session/<id>` |
| `turn.ended` | `{ turnId, stopReason }` | `session/<id>` |
| `prompt` | `{ turnId, text }` | `session/<id>` |
| `agent.message` | `{ turnId, messageId, delta }` (streamed) | `session/<id>` |
| `agent.plan` | `{ entries: { content, status }[] }` | `session/<id>` |
| `tool.call` | `{ toolCallId, action: Action, title }` | `session/<id>` |
| `tool.result` | `{ toolCallId, status, output? }` (output truncated, redacted) | `session/<id>` |
| `policy.decision` | `{ toolCallId, verdict, reason, source }` | `session/<id>` |
| `hook.run` | `{ hook, event, verdict?, durationMs, exitCode }` | `session/<id>` |
| `approval.requested` | `{ approvalId, action, reason }` | `session/<id>`, `approvals` |
| `approval.decided` | `{ approvalId, decision, remember? }` | `session/<id>`, `approvals` |
| `edit` | `{ path, kind, additions, deletions }` | `session/<id>` |
| `terminal.output` | `{ terminalId, chunk }` (streamed) | `session/<id>` |
| `checkpoint` | `{ checkpointId, turnId, ref }` | `session/<id>` |
| `review.thread` | `{ threadId, change: 'created' \| 'reply' \| 'resolved' \| 'outdated' }` | `session/<id>` |
| `usage` | `{ inputTokens, outputTokens, costUsd? }` (cumulative) | `session/<id>`, `board` |
| `error` | `ErrorData` | `session/<id>` |

Streamed kinds (`agent.message`, `terminal.output`) are coalesced in the JSONL log every 250 ms so the log stays readable; subscribers still get each chunk.

## Config file schemas

Each file has a JSON Schema generated from its Rust type and published at `docs/schemas/`. Unknown keys are an error, so typos surface immediately.

**`forge.toml`** (project root)

```toml
[project]
name = "forge"
default_agent = "claude"          # agent profile id

[[repo]]
name = "forge"
path = "."
base_branch = "main"

[budgets]
session_usd = 10.0                # pause the session above this
project_daily_usd = 50.0

[sessions]
max_concurrent = 5
worktree_retention_days = 7
```

**`.forge/agents/<id>.toml`** (agent profile)

```toml
agent = "claude-code"             # registry key
command = ["claude-code-acp"]     # overrides the registry default
model = "claude-opus-5"           # optional; passed if the agent supports it
mode = "accept-edits"
sandbox = "standard"
allow_unsandboxed = false
env_secrets = ["ANTHROPIC_API_KEY"]  # keyring names, injected via proxy

[network]
allow = ["api.anthropic.com", "crates.io", "static.crates.io", "registry.npmjs.org"]
```

**`policy.toml`** (managed, user, or `.forge/policy.toml`)

```toml
[[deny]]
exec = ["git push --force*", "git reset --hard origin*"]
reason = "History-rewriting"

[[ask]]
edit = ["**/migrations/**", ".github/workflows/**"]
reason = "Schema and CI changes need a human"

[[allow]]
exec = ["cargo test*", "cargo clippy*", "pnpm test*", "just *"]
modes = ["accept-edits", "auto"]

[protected]
paths = [".env*", "**/*.pem", "Cargo.lock"]   # edits denied unless the task says otherwise
```

Rule matching: `exec` patterns are globs over the space-joined argv; `edit` patterns are gitignore-style globs over repo-relative paths. A rule applies in every mode unless it lists `modes`.

## Hook I/O and plugin manifest

**Hook declaration** in `.forge/hooks.toml`:

```toml
[[hook]]
event = "tool.pre"
match = { action = "exec", argv = "npm install*" }   # optional filter
run = [".forge/hooks/block-npm.sh"]
timeout_ms = 5000
where = "sandbox"            # or "daemon"
```

**Input** (JSON on stdin):

```json
{
  "event": "tool.pre",
  "sessionId": "ses_01J...",
  "projectRoot": "/home/jeff/src/forge",
  "worktree": "/home/jeff/.local/share/forge/worktrees/forge/ses_01J.../forge",
  "mode": "accept-edits",
  "action": { "type": "exec", "argv": ["npm", "install", "left-pad"], "cwd": "." }
}
```

**Output** (JSON on stdout; all fields optional):

```json
{
  "verdict": "deny",
  "reason": "Use pnpm in this repo",
  "rewrite": null,
  "noteForAgent": "Run pnpm add instead."
}
```

Exit code 0 with empty output means no opinion. A non-zero exit or invalid JSON is logged and treated as no opinion, except `commit.pre` and `task.done`, where it blocks.

**Plugin manifest** `forge-plugin.toml` (v0.2, defined now so v0.1 hooks fit it):

```toml
[plugin]
name = "rust-guardrails"
version = "0.1.0"
forge = ">=0.2"

hooks = ["hooks.toml"]
skills = ["skills/*/SKILL.md"]
commands = ["commands/*.md"]
agents = ["agents/*.md"]
mcp = ["mcp.json"]
rules = ["rules/*.md"]

[[panel]]
name = "Crate graph"
entry = "panel/index.html"
scopes = ["project.read", "events.read"]
```

## Forge MCP tools

Forge's MCP server ships in v0.2, but its tool names and schemas are fixed now so the review store and LSP host expose the right data from v0.1.

| Tool | Input | Output |
| --- | --- | --- |
| `lsp_definition` | `{ path, line, column }` | `{ locations: { path, line, column }[] }` |
| `lsp_references` | `{ path, line, column, limit? }` | `{ locations: [...] }` |
| `lsp_hover` | `{ path, line, column }` | `{ markdown }` |
| `lsp_diagnostics` | `{ paths? }` | `{ diagnostics: { path, line, severity, message, source }[] }` |
| `lsp_rename_preview` | `{ path, line, column, newName }` | `{ edits: { path, hunks }[] }` (not applied) |
| `task_get` | `{}` | `{ title, spec, acceptanceChecks: string[] }` |
| `task_update_plan` | `{ entries: { content, status }[] }` | `{ approved: boolean }` (waits if approval required) |
| `review_threads` | `{ status? }` | `Thread[]` |
| `review_reply` | `{ threadId, body }` | `{}` |
| `checks_run` | `{ names? }` | `{ results: { name, passed, output }[] }` |
| `ask_human` | `{ question, options? }` | `{ answer }` (blocks until answered) |

Lines and columns are 1-based. Paths are relative to the session's working directory. Tool names use underscores because some agents reject dots in MCP tool names.

## Data model

One SQLite database per user at `~/.local/share/forge/forge.db` (macOS: `~/Library/Application Support/Forge/forge.db`), in WAL mode, with migrations in `crates/forged/migrations/` run on daemon start. The JSONL event logs remain the source of truth for events; SQLite indexes them.

```sql
CREATE TABLE projects (
  id TEXT PRIMARY KEY, name TEXT NOT NULL, root TEXT NOT NULL UNIQUE,
  created_at TEXT NOT NULL
);
CREATE TABLE repos (
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  name TEXT NOT NULL, path TEXT NOT NULL, base_branch TEXT NOT NULL,
  PRIMARY KEY (project_id, name)
);
CREATE TABLE sessions (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  title TEXT NOT NULL, agent_profile TEXT NOT NULL,
  state TEXT NOT NULL, mode TEXT NOT NULL, sandbox TEXT NOT NULL,
  branch TEXT NOT NULL, acp_session_id TEXT,
  input_tokens INTEGER NOT NULL DEFAULT 0, output_tokens INTEGER NOT NULL DEFAULT 0,
  cost_usd REAL, last_seq INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL, updated_at TEXT NOT NULL
);
CREATE TABLE worktrees (
  session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  repo_name TEXT NOT NULL, path TEXT NOT NULL,
  PRIMARY KEY (session_id, repo_name)
);
CREATE TABLE checkpoints (
  id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  n INTEGER NOT NULL, turn_id TEXT, created_at TEXT NOT NULL,
  UNIQUE (session_id, n)
);
CREATE TABLE checkpoint_refs (
  checkpoint_id TEXT NOT NULL REFERENCES checkpoints(id) ON DELETE CASCADE,
  repo_name TEXT NOT NULL, commit_sha TEXT NOT NULL,
  PRIMARY KEY (checkpoint_id, repo_name)
);
CREATE TABLE approvals (
  id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  action_json TEXT NOT NULL, reason TEXT NOT NULL,
  decision TEXT, remember TEXT, decided_at TEXT, created_at TEXT NOT NULL
);
CREATE TABLE review_rounds (
  session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  n INTEGER NOT NULL, checkpoint_id TEXT NOT NULL, created_at TEXT NOT NULL,
  PRIMARY KEY (session_id, n)
);
CREATE TABLE threads (
  id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  repo_name TEXT NOT NULL, path TEXT NOT NULL,
  start_line INTEGER NOT NULL, end_line INTEGER NOT NULL,
  checkpoint_id TEXT NOT NULL, status TEXT NOT NULL, created_at TEXT NOT NULL
);
CREATE TABLE comments (
  id TEXT PRIMARY KEY,
  thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
  author TEXT NOT NULL, body TEXT NOT NULL, created_at TEXT NOT NULL
);
CREATE TABLE events_index (
  session_id TEXT NOT NULL, seq INTEGER NOT NULL, kind TEXT NOT NULL,
  at TEXT NOT NULL, tool TEXT, path TEXT, byte_offset INTEGER NOT NULL,
  PRIMARY KEY (session_id, seq)
);
CREATE VIRTUAL TABLE events_fts USING fts5(session_id UNINDEXED, seq UNINDEXED, text);
```

**On disk**

| Path | Contents |
| --- | --- |
| `<project>/.forge/` | Checked-in config, tasks, hooks (see Config in component-architecture.md) |
| `<project>/.forge/sessions/<id>/events.jsonl` | Event log (gitignored) |
| `<data>/worktrees/<project>/<session>/<repo>/` | Session worktrees |
| `<data>/servers/` | Installed language servers |
| `<data>/agents/` | Agent adapter installs |
| `$XDG_RUNTIME_DIR/forge/forged.sock` (macOS: `<data>/forged.sock`) | API socket, mode 0600 |

`<data>` is `~/.local/share/forge` on Linux and `~/Library/Application Support/Forge` on macOS.
