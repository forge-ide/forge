# Window Hierarchy

> Extracted from CONCEPT.md — Dashboard window, Session window, and Command palette

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

### 3.2 Session window
Default: one window per running session the user is actively viewing. Holds the session's pane layout (§4). Spawned from the dashboard, from the CLI (`forge open .`), or from a file manager.

**User preference:** the setting `windows.session_mode` can be set to `tabbed`, which puts multiple sessions in a single window as tabs (like browser tabs). Default is `window-per-session`. The preference is global, not per-session — users pick one mental model.

### 3.3 Command palette / quick picker
`Cmd/Ctrl+K` (primary) or `Cmd/Ctrl+Shift+P` (alternate, for users arriving from VS Code muscle memory). Addresses every Forge action: session switching, file/line jumps, provider/model switching, skill toggles, MCP commands, agent invocation, settings navigation, terminal commands via `>`, workspace tasks from `Makefile`/`justfile`. Context-aware — in a session window, file-jump defaults to workspace files; in the dashboard, to sessions.

F-157 lands the base infrastructure: the overlay component, a module-scoped `registerCommand({ id, title, run })` registry, fuzzy filtering, and the first built-in entry (`Open Agent Monitor`). The larger set of actions above lands incrementally as each action's host module calls `registerCommand`.

**Explicitly not windows:** Settings (lives in the dashboard), Git (session sidebar), Extensions (dashboard catalog).
