# Window Hierarchy

> Extracted from CONCEPT.md — Dashboard window, Workspace window, and Command palette

---

## 3. Window hierarchy

Two window types. Plus a palette overlay.

### 3.1 Dashboard window (primary)
Home screen. Opens by default on app launch. Shows live state:
- **Sessions** — active, recent, archived. One-click resume. Active/archived filter.
- **Providers** — connection state, quick-enable toggles, credential status
- **Skills / MCPs / Agents** — catalog, enable per-workspace, inspect
- **Usage** — tokens and cost by provider, project, and time window
- **Containers** — OCI-managed sandboxes, health, logs
- **Config** — settings, keybindings, theme, telemetry

The dashboard holds live state, not launchers — you can tail a running session, kill a runaway agent, or tweak a provider mid-flight from here.

### 3.2 Workspace window
Default: one window per workspace. Holds every session belonging to that workspace, with sessions exposed as sub-routes inside the window (the session's pane layout, §4, renders inside the workspace shell). Spawned from the dashboard, from the CLI (`forge open .`), or from a file manager.

Window labels follow the format `workspace-{workspace_id}`. Sessions no longer get their own OS-level window — opening a session navigates to its sub-route inside the workspace window that owns it. This consolidates per-workspace resources (LSP servers, terminal sessions, file-tree caches) into a single webview and avoids the resource cost of one window per concurrent session.

**Historical note:** earlier milestones used a `session-{session_id}` window-per-session model with a `windows.session_mode = tabbed | window-per-session` user preference. That model was retired in favor of the workspace-per-window scheme; the setting is no longer consulted.

### 3.3 Command palette / quick picker
`Cmd/Ctrl+K` (primary) or `Cmd/Ctrl+Shift+P` (alternate, for users arriving from VS Code muscle memory). Addresses every Forge action: session switching, file/line jumps, provider/model switching, skill toggles, MCP commands, agent invocation, settings navigation, terminal commands via `>`, workspace tasks from `Makefile`/`justfile`. Context-aware — in a session window, file-jump defaults to workspace files; in the dashboard, to sessions.

F-157 lands the base infrastructure: the overlay component, a module-scoped `registerCommand({ id, title, run })` registry, fuzzy filtering, and the first built-in entry (`Open Agent Monitor`). The larger set of actions above lands incrementally as each action's host module calls `registerCommand`.

**Explicitly not windows:** Settings (lives in the dashboard), Git (session sidebar), Extensions (dashboard catalog).
