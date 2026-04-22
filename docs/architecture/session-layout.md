# Session Layout

> Extracted from CONCEPT.md — pane types, layout rules, and CLI surface for sessions

---

## 4. Session layout

Sessions use standard editor layout semantics. There is no fixed layout; users compose panes like any modern editor.

### 4.1 Pane types

Source of truth: `PaneType` in `crates/forge-shell/src/ipc.rs`. Five variants, persisted on every leaf in `.forge/layouts.json` and carried on the IPC contract. An unknown variant fails deserialization loudly, so a new pane type must land as an enum variant before it can ship.

| Pane type | Variant (`pane_type`) | Purpose |
|---|---|---|
| **Chat** | `chat` | Agent conversation. Streaming, tool calls inline, sub-agent banners, `@`-context. |
| **Terminal** | `terminal` | Shell emulation via Ghostty's VT library. Shared or agent-controlled. |
| **Editor** | `editor` | Buffer on disk or in memory. Monaco in a webview (see §7). |
| **Files** | `files` | File tree surfaced as a main-area pane. Distinct from the activity-bar sidebar file tree — the sidebar remains the primary entry point (toggle: `Cmd/Ctrl+Shift+E`); this variant exists for users who want the tree docked inside the pane grid. |
| **Agent monitor** | `agentmonitor` | Three-column agent monitor view (F-140): agent list, trace timeline, inspector. Dockable so multi-agent workflows don't compete with the chat pane for space. |

### 4.2 Layout rules
- Standard splits: single, horizontal, vertical, grid
- No hard cap on pane count — users split as needed
- Any pane can hold any of the five `PaneType` variants; pane type is not fixed at creation
- Panes can be resized; minimum width 320px (below that, pane header collapses to icons)
- Drag any pane to any edge of another pane to re-dock (including effectively sidebar-sized positions)
- Layouts persist per-workspace in `.forge/layouts.json`; see §4.1 for the enum that gates what a leaf may hold

### 4.3 Default layout
A new session opens with **a single chat pane** filling the main area. When the user opens a file (from the files sidebar, `@`-ref click, or a tool call that reads/edits one), an editor pane opens via a sensible split — first file splits right; subsequent files become tabs in that editor pane.

### 4.4 Pane chrome rules (from DESIGN.md)
- Every pane header shows its active provider dot where applicable
- Panes are always labeled; no anonymous panes
- Pane-local actions live in the pane header
- Streaming state belongs to the chat pane; other panes never show the ember streaming cursor

### 4.5 Persistent schema

Source of truth: `crates/forge-shell/src/ipc.rs` (`Layouts`, `Layout`, `LayoutTree`, `PaneState`, `SplitDirection`, `PaneType`). Shapes are ts-rs-exported to `web/packages/ipc/src/generated/` — the TS and Rust sides evolve in lockstep.

**`Layouts`** — the on-disk root of `.forge/layouts.json`.

| Field | Type | Semantics |
|---|---|---|
| `active` | `string` | Key into `named` the UI restores on next session open. |
| `named` | `map<string, Layout>` | Named layouts sharing the workspace (e.g. `"default"`, `"split-editor"`). |

**`Layout`** — one named layout.

| Field | Type | Semantics |
|---|---|---|
| `tree` | `LayoutTree` | The serialized pane tree. |
| `pane_state` | `map<string, PaneState>` | Side-car state keyed by leaf `id`. Orphan keys are garbage-collected by the frontend without a schema change. Defaults to `{}` on load. |

**`LayoutTree`** — discriminated union on `kind` (`"leaf" | "split"`), mirroring the TS `LayoutNode` shape.

| Variant | Fields | Semantics |
|---|---|---|
| `leaf` | `id: string`, `pane_type: PaneType` | A terminal node. `id` is stable across sessions so `pane_state` keys stay valid after tree edits. Unknown `pane_type` values fail deserialization loudly — a future pane type must land as a `PaneType` variant before it can be persisted. |
| `split` | `id: string`, `direction: SplitDirection`, `ratio: f32`, `a: LayoutTree`, `b: LayoutTree` | An internal node. `ratio` is the fraction of the container occupied by `a`, in `0.0..=1.0`. |

**`PaneState`** — per-leaf runtime state. All fields optional so each pane type persists only what's meaningful.

| Field | Type | Semantics |
|---|---|---|
| `active_file` | `string?` | Editor panes: last-focused file path, relative to workspace root. `None` on panes that don't address a file. |
| `scroll_top` | `i64?` | Editor / chat scroll-back panes: top scroll offset in pixels (rounded). `None` when unknown or inapplicable. |
| `terminal_pid` | `u32?` | Terminal panes: PID of the live child shell. Carried through restart so the UI can re-attach rather than spawn a new PTY. |

**`SplitDirection`** — `"h" | "v"`. `h` renders children side-by-side; `v` stacks them.

**`PaneType`** — `"chat" | "terminal" | "editor" | "files" | "agentmonitor"`. Same five variants enumerated in §4.1; a typed enum (not a free-form string) so unknown values fail loudly on load.

#### Silent-default fallback

`read_layouts` degrades to `Layouts::default()` — a single `chat` leaf with id `"root"` under the `"default"` key — on any of:

- file missing (fresh workspace, first session open);
- file unreadable (permissions anomaly);
- file present but invalid JSON (user hand-edit, crash-during-write, or a forward-incompatible variant).

A surfaced error would leave the webview with no layout to mount and the user with a blank window. Losing the persisted layout is recoverable; losing the ability to open the session is not. See `load_layouts_from_disk` in `crates/forge-shell/src/ipc.rs`.

---

## 5. CLI surface

Sessions are independently invokable. This is non-negotiable — it's what makes Forge different from a chat app, and what makes headless/CI use possible.

### 5.1 Session lifecycle

```
forge session new agent <name>    [--workspace PATH] [--ephemeral]
forge session new provider <spec> [--workspace PATH] [--ephemeral]
forge session list                [--archived] [--all]
forge session attach <id>         # reactivates archived sessions on demand
forge session tail <id>           # streaming transcript to stdout
forge session kill <id>
forge session export <id>         [--format jsonl|md]
```

### 5.2 Headless execution

```
forge run agent <name>    [--input -|PATH] [--output -|PATH]
forge run provider <spec> [--input -|PATH] [--output -|PATH]
```

One-shot runs. Creates an ephemeral session by default, streams events to stdout, exits with the session's exit code. Enables CI use, shell piping, cron jobs. The subcommand grammar matches `forge session new` intentionally — `agent <name>` and `provider <spec>` mean the same thing in both places.

### 5.3 Workspace entry point

```
forge open <path>
```

GUI-coupled. Opens the workspace in a session window. If a session already exists in that workspace, attaches to the most recent active one. If none, prompts in-GUI to create one. Distinct from `forge session new` because it has GUI semantics and reuse behavior.

### 5.4 Catalog management

```
forge skill    list | install <git-url|path> | update <name> | enable | disable
forge mcp      list | test <name> | restart <name> | import [--from vscode|cursor|...]
forge provider list | login <id> | logout <id>
forge container list | exec <id> | rm <id>
```

`forge mcp import` is a one-shot migration utility: reads another tool's MCP config (VS Code, Cursor, Claude Desktop, Continue, Kiro, Codex) and converts it to the universal `.mcp.json` schema.

### 5.5 Session persistence

Sessions have a **persistence mode**:
- `persist` (default) — on end, the session is automatically archived. Transcript preserved. Reactivatable via `forge session attach`.
- `ephemeral` — on end, the process is killed and all data is purged from disk.

Controlled by `sessions.persistence` in user config, overridable per-workspace, or per-session via `--ephemeral` at start. `forge run` sessions are ephemeral by default.

Active session count is soft-limited (default 10, configurable via `sessions.active_limit`). Archived sessions have no limit. Dashboard session list defaults to active; archived accessible via filter. Creating a new session when at the active limit prompts the user to archive the oldest, cancel, or raise the limit.

### 5.6 Credentials

```
forge provider login <id>    # reads API key from stdin
forge provider logout <id>
```

Stdin-only in v1. Browser-OAuth flow deferred to v1.1. Forge also reads standard environment variables (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, etc.) as a fallback for users who already have these in their shell.
