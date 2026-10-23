# Interfaces and data

The contracts every crate and the UI build against. Rust types in `forge-api` are the source of truth; this file is the human-readable version.

## Conventions

- **IDs** are ULIDs with a type prefix: `prj_`, `ses_`, `trn_` (turn), `chk_` (checkpoint), `apr_` (approval), `thr_` (thread), `cmt_` (comment), `hnk_` (hunk), `sub_` (event subscription). Events use a per-session sequence number instead, plus a daemon-wide stream cursor on notifications.
- **Wire format:** JSON with camelCase fields. Rust types use snake_case with `#[serde(rename_all = "camelCase")]`.
- **Times** are RFC 3339 in UTC. **Paths** on the wire are repo-relative with forward slashes; absolute paths never cross the API, with two exceptions: `Project.root` and the `path` param of `project.open`, which identify the project on the user's machine.
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
type CollisionResolution = 'reuse-branch' | 'adopt-worktree';
type BranchCollision = { repo: string;
  kind: 'branch-exists' | 'worktree-exists' | 'owned-by-live-session'; sessionId?: string };
```

| Method | Params | Result |
| --- | --- | --- |
| `server.hello` | `{ client, apiVersion }` | `{ daemonVersion, apiVersion, platform, sandboxTiers }` |
| `project.open` | `{ path }` | `Project` |
| `project.list` | `{}` | `Project[]` |
| `project.get` | `{ projectId }` | `Project` |
| `agent.list` | `{}` | `AgentProfile[]` |
| `session.create` | `{ projectId, agentProfile, title, prompt, mode, sandbox?, branch?, onCollision? }` | `Session` |
| `session.branchPreview` | `{ projectId, title, branch? }` | `{ branch, collisions: BranchCollision[] }` |
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

Notifications arrive as `event` with `{ subscriptionId, topic, seq, event }`, where `seq` is the stream cursor (see `events.subscribe`). Topics: `board`, `approvals`, `session/<id>`, `project/<id>`.

Method notes:

- **`server.hello`**: `sandboxTiers` lists the tiers this machine can run, as probed when the daemon started, plus `unsandboxed`, which always runs but still needs each profile's opt-in. Clients compare the major `apiVersion` and refuse to continue on a mismatch.
- **`project.open`**: `path` must be absolute (clients resolve relative paths). If the directory has no `forge.toml`, it must be a git repository root on a branch; the daemon writes a single-repo `forge.toml` (name from the directory, `path = "."`, `base_branch` from the current branch) and never overwrites an existing one. Reopening the same root returns the same project ID and picks up edits to `forge.toml`. Invalid config is `ValidationFailed` with every problem listed.
- **`session.create`**: returns at once in `provisioning`; progress arrives as events, starting with `session.state` to `ready` (or `failed`, with the reason). The agent profile is `.forge/agents/<agentProfile>.toml` (`NotFound` if missing). The sandbox tier is `sandbox`, else the profile's, else `standard`; a tier this machine can't run is `SandboxUnavailable`, never a silent fallback, and `unsandboxed` also needs `allow_unsandboxed = true` in the profile. The branch is `branch`, or one derived from `title` (see `session.branchPreview`). If the branch or its worktree already exists in any repo, the call fails with `Conflict` unless `onCollision` says what the user chose: `reuse-branch` (only if no worktree has the branch checked out) or `adopt-worktree` (only if git lists it as a worktree of that branch). A worktree owned by a live session is always `Conflict`. A repo path that resolves outside the project root is `ValidationFailed`. More live sessions than `sessions.max_concurrent` is `InvalidState`. A non-empty `prompt` starts the first turn once the session is ready.
- **`session.branchPreview`**: the branch name `session.create` would use and what already holds it in each repo, so a client can show the name and ask about collisions first. A derived name uses the `[branches]` format and types from `forge.toml`, falling back to the user config, then `{type}/{slug}` with `feat`, `fix`, `bug`. An invalid branch name, or a title with nothing to make a slug from, is `ValidationFailed`.
- **`session.prompt`**: accepted in `ready`, `review`, and `paused` (`InvalidState` otherwise); returns once the turn has started. **`session.cancel`**: only in `working`; the turn ends with stop reason `cancelled` and the session returns to `ready`. **`session.close`**: in `ready`, `review`, `paused`, or `failed`; stops the agent, keeps the worktrees, and moves to `done`.
- **`events.subscribe`**: `topics` must be non-empty, and `project/<id>` and `session/<id>` must exist (`NotFound` otherwise). A connection can only unsubscribe its own subscriptions, and they end when it disconnects. Notification `seq` is the daemon-wide stream cursor, not the event's own `seq`: it increases across all sessions and topics and survives restarts. With `afterSeq`, every matching event after that cursor is replayed first, then live events follow with none missed or repeated; replayed events arrive after the subscribe response. An `afterSeq` ahead of the stream (for example, from before the data directory was replaced) is `ValidationFailed`; the client resubscribes without it and reloads. Each event reaches a subscription once, naming the first of its topics that the subscription includes.
- **`events.query`**: `limit` is 1 to 1000. Returns the newest matching events below `before`, oldest first. `text` matches as a phrase.
- **Methods not yet implemented** return the standard JSON-RPC `-32601` (method not found) with a message naming the method.

## Events

Every event has the envelope `{ seq, sessionId, at, kind, parent?, body }`. `parent` links a decision, hook run, or result to the tool call it belongs to.

| Kind | Body | Topic |
| --- | --- | --- |
| `session.state` | `{ from, to, reason? }` | `session/<id>`, `project/<id>`, `board` |
| `turn.started` | `{ turnId, source: 'human' \| 'review' \| 'hook' }` | `session/<id>` |
| `turn.ended` | `{ turnId, stopReason }` | `session/<id>` |
| `prompt` | `{ turnId, text }` | `session/<id>` |
| `agent.message` | `{ turnId, messageId, delta }` (streamed) | `session/<id>` |
| `agent.plan` | `{ entries: { content, status }[] }` | `session/<id>` |
| `tool.call` | `{ toolCallId, action: Action, title }` | `session/<id>` |
| `tool.result` | `{ toolCallId, status, output? }` (output truncated, redacted) | `session/<id>` |
| `policy.decision` | `{ toolCallId, verdict, reason, source }` | `session/<id>` |
| `hook.run` | `{ hook, event, verdict?, durationMs, exitCode }` | `session/<id>` |
| `approval.requested` | `{ approvalId, action, reason }` | `session/<id>`, `project/<id>`, `approvals` |
| `approval.decided` | `{ approvalId, decision, remember? }` | `session/<id>`, `project/<id>`, `approvals` |
| `edit` | `{ path, kind, additions, deletions }` | `session/<id>` |
| `terminal.output` | `{ terminalId, chunk }` (streamed) | `session/<id>` |
| `network` | `{ host, port, verdict: 'allow' \| 'deny', reason?, bytesSent, bytesReceived, durationMs }` (one per connection through the egress proxy: when it closes, or at once when denied) | `session/<id>` |
| `checkpoint` | `{ checkpointId, turnId, ref }` | `session/<id>` |
| `review.thread` | `{ threadId, change: 'created' \| 'reply' \| 'resolved' \| 'outdated' }` | `session/<id>` |
| `usage` | `{ inputTokens, outputTokens, costUsd? }` (cumulative) | `session/<id>`, `project/<id>`, `board` |
| `error` | `ErrorData` | `session/<id>` |

Streamed kinds (`agent.message`, `terminal.output`) are logged one chunk per event, exactly as subscribers receive them, so replay matches live delivery. Coalescing them in the JSONL log (every 250 ms, for readability) is planned; it must keep each chunk's `seq` and stream cursor so replay stays lossless.

`project/<id>` carries the `board` and `approvals` kinds for that project's sessions.

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

[branches]                        # overrides the user config's [branches], per key
format = "{type}/{slug}"
types = ["feat", "fix", "bug"]
```

`[budgets]` keys are optional (unset means no cap); `[sessions]` defaults to the values shown. Repo paths must be relative and stay inside the project root, even after following symlinks. `base_branch` must be a branch name: no spaces and no leading `-`. `[branches]` is optional: `format` must contain `{slug}` and may contain `{type}`; `types` are lowercase letters, digits, and hyphens, and the first is used when a title names none.

**`~/.config/forge/config.toml`** (user config; every key optional)

```toml
default_agent = "claude"          # used when neither session nor project names one

[ui]
theme = "system"                  # system | light | dark

[branches]                        # how session branches are named
format = "{type}/{slug}"          # e.g. "jeff/{type}/{slug}"
types = ["feat", "fix", "bug"]

[sandbox]
read_only = ["/home/me/.local/share/agents"]  # extra paths visible read-only in every sandbox
```

`sandbox.read_only` entries must be absolute and not `/`; a session fails to start if one is the home dir itself or contains Forge's data or runtime dir. They live here rather than in agent profiles, which a cloned repo controls.

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

`sandbox` defaults to `standard`; `allow_unsandboxed` defaults to `false`. `[network] allow` lists the hosts the session's egress proxy lets through, on ports 80 and 443: plain host names, matched exactly (ignoring case and a trailing dot), with no scheme, port, path, wildcard, or IP address; anything else fails `session.create` with `ValidationFailed`. An empty or missing list means no network. `env_secrets` may name only secrets Forge knows how to inject (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GEMINI_API_KEY`; anything else is `ValidationFailed`). Each is read at launch from the OS keyring, service `forge`, account the secret's name; one that is missing fails the session with the command that stores it (`secret-tool store --label='Forge NAME' service forge account NAME` on Linux, `security add-generic-password -s forge -a NAME -w` on macOS). The profile ID is the file name and may use only letters, digits, `-`, and `_`.

**`policy.toml`** (managed: `/etc/forge/managed.toml` on Linux, `/Library/Application Support/Forge/managed.toml` on macOS; user: `~/.config/forge/policy.toml`; project: `<root>/.forge/policy.toml`)

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

Every key is optional; a missing file has no rules. Each rule needs at least one `exec` or `edit` pattern; `modes`, if present, must be non-empty.

Rule matching: `exec` patterns are globs over the space-joined argv, with the program reduced to its file name; `*` matches any run of characters (including `/` and spaces), and `\*` and `\\` are a literal `*` and `\`. Allow rules match only a single simple command; deny and ask rules match any simple command in a line, and ask instead when only an expansion could make them match (component-architecture.md, Policy engine). `edit` and `protected` patterns are gitignore-style globs over repo-relative paths: a pattern without a `/` (other than a trailing one) matches at any depth, one with a `/` or a leading `/` is anchored at the repo root, `*` and `?` stop at `/` while `**` crosses directories, a pattern matching a directory matches everything under it, and `!` negation is not supported; matching ignores case on macOS. Patterns may not use `..`. A rule applies in every mode unless it lists `modes`. Protected paths add to the built-in list (`.env`, `.env.*`, `*.pem`, `*.key`, `.forge/`). Lower layers only tighten: a deny or ask in any layer beats an allow in any other.

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
  "worktree": "/home/jeff/src/forge/.worktrees/feat/retry-backoff",
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

Forge's MCP server ships in v0.2, but its tool names and schemas are fixed now so the review store and LSP host expose the right data from v0.1. v0.1 ships a minimal per-session endpoint with `git_commit` only, so agents have a commit path.

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
| `git_commit` | `{ message, paths? }` | `{ commit, branch }` (all changes, including untracked files, when `paths` is omitted; policy-checked, runs the `commit.pre` hook) |

Lines and columns are 1-based. Paths are relative to the session's working directory. Tool names use underscores because some agents reject dots in MCP tool names.

## Data model

One SQLite database per user at `~/.local/share/forge/forge.db` (macOS: `~/Library/Application Support/Forge/forge.db`), in WAL mode, with migrations in `crates/forged/migrations/` run on daemon start. The JSONL event logs remain the source of truth for events; SQLite indexes them.

The schema below is the v0.1 target. Each milestone adds the tables it uses as a new numbered migration (M1 adds `projects` and `repos`; M2 adds `sessions`, `worktrees`, `events_index`, and `events_fts`); progress is tracked in `PRAGMA user_version`, and a daemon refuses to start on a database newer than it knows.

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
CREATE INDEX sessions_by_project ON sessions (project_id, state);
CREATE TABLE worktrees (
  session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  repo_name TEXT NOT NULL, path TEXT NOT NULL,
  PRIMARY KEY (session_id, repo_name)
);
CREATE INDEX worktrees_by_path ON worktrees (path);
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
  stream_seq INTEGER NOT NULL UNIQUE,   -- daemon-wide cursor; see events.subscribe
  PRIMARY KEY (session_id, seq)
);
CREATE VIRTUAL TABLE events_fts USING fts5(session_id UNINDEXED, seq UNINDEXED, text);
```

**On disk**

| Path | Contents |
| --- | --- |
| `<project>/.forge/` | Checked-in config, tasks, hooks (see Config in component-architecture.md) |
| `<project>/.forge/sessions/<id>/events.jsonl` | Event log, one event per line with its `streamSeq` (ignored by `.forge/sessions/.gitignore`) |
| `<repo>/.worktrees/<branch>/` | Session worktrees (excluded in `.git/info/exclude`) |
| `<data>/servers/` | Installed language servers |
| `<data>/agents/` | Agent adapter installs |
| `$XDG_RUNTIME_DIR/forge/forged.sock` (macOS: `<data>/forged.sock`) | API socket, mode 0600 |

`<data>` is `~/.local/share/forge` on Linux (or `$XDG_DATA_HOME/forge`) and `~/Library/Application Support/Forge` on macOS. The daemon log is `<data>/forged.log` when the app starts the daemon. `FORGE_DATA_DIR`, `FORGE_RUNTIME_DIR` (the socket's directory), and `FORGE_CONFIG_DIR` override the data, socket, and user config locations, and `FORGE_MANAGED_POLICY` the managed policy file; tests always set them. The managed policy constrains agents, not the user, who controls the daemon's environment.
