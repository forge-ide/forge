# Layout & Panes

> Extracted from SPECS.md §3 — single/H/V/grid splits, pane types, pane header, drag-to-dock, minimum width behavior, and layout persistence

---

## 3. Session layout and panes

**Purpose.** The area that holds one or more panes (chat, terminal, editor) using standard editor layout semantics.

**Size.** Fills the main canvas area left of the sidebar, below the tab bar, above the status bar.

### 3.1 Layout model

- **Single pane** — one pane fills the entire main area (the default for a new session, holding chat)
- **Horizontal split** — two or more panes stacked top to bottom
- **Vertical split** — two or more panes side by side (the most common multi-pane layout)
- **Grid** — arbitrary combinations of H and V splits
- **No hard cap** on pane count. Users add splits as needed, VS Code-style.

Gridlines are 1px in `--color-border-1`. Dragging resizes in 4px snaps; Alt+drag is unsnapped. Double-clicking a gridline resets to balanced split.

### 3.2 Pane types
Five types: `chat`, `terminal`, `editor`, `files`, `agentmonitor`. Each pane header shows its type. The authoritative list is the `PaneType` enum in `crates/forge-shell/src/ipc.rs`; see `docs/architecture/session-layout.md §4.1` for the full variant table.

The activity-bar sidebar file tree is independent of the `files` pane variant — the sidebar is part of the shell chrome (`Cmd/Ctrl+Shift+E` toggles it), while the `files` pane is a main-area dockable variant for users who want the tree inside the pane grid.

### 3.3 Pane header (28px)
- Left: type label in `--label` style (mono xs, uppercase, tracked) — e.g. `CHAT`, `TERMINAL`
- Then: subject name — chat agent name, filename, shell name — with provider dot if relevant
- Right: pane-local actions (max 3 visible), then overflow `⋯`

### 3.4 Default layout
New session opens as **a single chat pane** filling the main area.

When the user opens a file (from the files sidebar, `@`-ref click, or a tool call that reads/edits one):
- If there is no editor pane yet: split right to create an editor pane (50/50)
- If there is already an editor pane: open the file as a new tab in that pane

Same logic applies to the terminal (`Cmd/Ctrl+` ` ` ` opens a terminal pane via split down).

### 3.5 Split controls
Live in the rightmost portion of the tab bar:
- Split right (`⬈`) — moves the active tab's pane into a new pane on the right
- Split down (`⬊`) — same, below
- Reset layout (`⊞`) — restores the default single-chat layout

### 3.6 Drag-to-dock
Dragging any pane by its header onto another pane's edge re-docks it:
- Drop on right/left edge: vertical split
- Drop on top/bottom edge: horizontal split
- Drop on center: move as a tab into the target pane (if both panes are the same type)

The drop zones highlight with `rgba(255,74,18,0.12)` during drag.

### 3.7 Minimum width behavior
Panes have a minimum width of 320px. Below that:
- Pane header labels collapse to icons only
- Chat pane: tool-call cards render with truncated arg summaries; ctx chips wrap rather than scroll
- Editor pane: minimap auto-hides
- Terminal pane: no change (terminals handle narrow widths natively)

### 3.8 Layout persistence
`.forge/layouts.json` stores the pane tree per session. On reattach, the layout is restored exactly. When running `tabbed` mode, each session keeps its own layout; switching tabs restores.

**Doesn't do.**
- Does not support floating/detached panes (v1 scope). Use a second window for that.
- Does not auto-hide panes when inactive. Users explicitly close panes.
