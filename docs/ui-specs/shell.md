# Session Window Shell

> Extracted from SPECS.md §2 — 44px activity bar, 32px title bar, 22px status bar, window structure and behaviours

---

## 2. Session window shell

**Purpose.** The container for a single session. Holds title bar, activity bar, sidebar, main pane area, and status bar.

**Size.** Default 1280×800 on first open, minimum 960×600, remembered per session.

**Structure.**
```
┌─── title bar (32) ──────────────────────────────────┐
│ ●●● │ session-name · workspace · #id  │ detach etc │
├──┬──┬───────────────────────────────────────────────┤
│  │  │  tab bar (33)                                 │
│  │  ├───────────────────────────────────────────────┤
│AB│SB│                                               │
│44│~ │  main pane area                               │
│  │  │  (one or more panes, composed via standard    │
│  │  │   splits — see §3)                            │
├──┴──┴───────────────────────────────────────────────┤
│ status bar (22) — always ember                      │
└─────────────────────────────────────────────────────┘
```
AB = activity bar (44px). SB = sidebar (files tree or other activity-bar content). Sidebar is togglable per activity-bar tab; width remembers across sessions.

**Required state.**
- Connection status to session process (connecting / connected / disconnected)
- Current pane layout (persisted to `.forge/layouts.json`)
- Active tab per pane
- Unsaved dirty indicators
- Background agent count (surfaces as a small badge in status bar)

**Behaviours.**
- Title bar shows `<session name>` in Fira Code 11px, followed by separator `·`, workspace name, then the short session id in tertiary text.
- Right-side title bar actions: `detach`, `share`, window controls. `detach` disconnects the GUI but leaves the session running.
- On GUI quit, sessions survive. A toast appears at next launch: `N sessions still running. Reattach?`

**Window mode toggle.** The setting `windows.session_mode` controls whether this is a single-session window or a tabbed multi-session window. The default is single-session. In `tabbed` mode, a session tab strip appears above the pane tab bar; switching tabs swaps the main pane area's contents entirely.

**Doesn't do.**
- Does not embed settings (those live in the dashboard).
- In default mode, does not host multiple sessions in one window. One session per window; multiple windows allowed.
