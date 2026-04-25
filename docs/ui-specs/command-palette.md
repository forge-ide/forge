# Command Palette

> Extracted from F-157 and `docs/architecture/window-hierarchy.md §3.3` — keyboard-invoked overlay that lists every command registered via the module-scoped registry.

---

## CP. Command palette

**Purpose.** One keyboard surface that addresses every Forge action — session switching, file/line jumps, provider/model switching, skill toggles, MCP commands, agent invocation, settings navigation. The palette owns no commands itself; it is purely a UI surface over the `registerCommand({ id, title, run })` registry in `web/packages/app/src/commands/registry.ts`.

**Where.** Global overlay, above all windows. Mounted once at the app root (`App.tsx`) and rendered into a fixed-position backdrop spanning the viewport.

**Trigger.** Two shortcuts, both bound at the `window` level in capture phase so monaco / xterm input cannot swallow them:

- `Cmd/Ctrl+K` — primary, matches `docs/architecture/window-hierarchy.md §3.3`.
- `Cmd/Ctrl+Shift+P` — alternate, for users arriving from VS Code muscle memory.

A second press of either shortcut while open closes the palette (toggle).

**Size.** `min(560px, 90vw)` wide, `max-height: 60vh`, anchored 96px from the top of the viewport on a `rgba(0,0,0,0.35)` backdrop.

### CP.1 Structure

```
┌──────────────────────────────────────────────┐
│ Type a command…                              │  ← search input, mono 13px
├──────────────────────────────────────────────┤
│ ▸ Open Agent Monitor                         │  ← active row, ember left context
│   Switch Provider                            │
│   Open Settings                              │
│   …                                          │
└──────────────────────────────────────────────┘
```

The palette is a single column: a search input (`--font-mono` 13px) with a 1px `--color-border-1` bottom divider, and a scrollable result list below. Background `--color-surface-2`, 1px `--color-border-2` border, radius `--r-lg`, drop shadow `0 24px 48px rgba(0,0,0,0.5)`.

### CP.2 Search input

- Placeholder: `Type a command…` (sentence case, terminal ellipsis, per `voice-terminology.md`).
- Background `--color-surface-1`, padding `var(--sp-3) var(--sp-4)`, no inner border, focus ring `2px solid var(--color-ember-400)` with 2px offset.
- Echoes the live query; the palette re-evaluates the result list on every input.

### CP.3 Result rendering

Each row is a `<li role="option">`:

- `--font-body` 13px, color `--color-text-primary`, padding `var(--sp-2) var(--sp-4)`.
- Active row (cursor target): background `--color-surface-3`. Hover: same.
- Focus: `2px solid var(--color-ember-400)` outline with 2px offset.
- Title only in v1 — no kbd hint, no group header, no icon. Future fields (kbd shortcut, source module, accent dot for provider commands) plug into the row's flex slot to the right of the title without changing the existing title typography.

The list is sorted by descending fuzzy score (case-insensitive subsequence with prefix and contiguity bonuses; see `registry.ts → fuzzyMatch`). Ties resolve by registration order. An empty query renders all commands in registration order.

### CP.4 States

The palette renders the four `component-principles.md` states distinctly:

- **Closed.** Not in the DOM. The shortcut listener is the only resident artifact.
- **Idle (open, empty query).** Input focused, full registry listed in registration order, cursor on row 0.
- **Results (open, query matches).** Filtered list sorted by score, cursor on row 0; row 0 resets on every query change.
- **Empty (open, query has no matches).** Single non-interactive row reading `// no matching commands` (mono-comment placeholder per `voice-terminology.md §8`). Enter is a no-op.

The palette has no async loading or error states — the registry is in-process and synchronous. Commands that themselves do async work own their own loading/error surfaces after `run()` fires.

### CP.5 Keyboard model

- `↑` / `↓` — move the cursor within the result list. Wraps at both ends.
- `Enter` — close the palette, then invoke the active row's `run()`. Closing first guarantees the handler (which may navigate) runs against a clean DOM.
- `Esc` — close. Query is discarded.
- `Cmd/Ctrl+K` or `Cmd/Ctrl+Shift+P` while open — close (toggle).
- Mouse: clicking a row activates it and runs in one gesture; clicking the backdrop closes.

DOM focus stays in the input while the palette is open; arrow navigation does not move DOM focus. The active row is signalled via `aria-selected="true"`.

### CP.6 Interaction with the command registry

The palette reads the live registry on every open and on every query change — late `registerCommand` calls (e.g., a route-scoped registration that lands after the user opens the app) surface immediately on the next open. The palette never caches the list across opens.

Built-in commands register from `registerBuiltins()` inside the `<Router>` subtree (today: `Open Agent Monitor` → `/agents`). Feature modules register their own actions via `registerCommand` during component mount and dispose on unmount.

### CP.7 Accessibility

- Root dialog: `role="dialog"`, `aria-modal="true"`, `aria-label="Command palette"`.
- Result list: `role="listbox"`; rows: `role="option"` with `aria-selected` reflecting the cursor.
- Empty-state row: `aria-disabled="true"` so AT does not present it as actionable.
- Focus is trapped to the dialog while open (`useFocusTrap`); closing restores focus to the previously focused element.
- The shortcut listener calls `preventDefault`, so the host browser's `Cmd+K` (search) does not also fire.

### CP.8 Cross-spec references

- `docs/architecture/window-hierarchy.md §3.3` — palette charter and the long-form list of commands the registry will eventually carry.
- `voice-terminology.md §8` — placeholder, empty-state, and command-title voice rules.
- `component-principles.md` — the four-state model the palette renders against.

**Doesn't do.**
- Does not host commands itself — every entry comes from `registerCommand`.
- Does not group results by module or category in v1 — score-ordered flat list only.
- Does not persist a recents / pinned list — every open starts from registration order.
- Does not expose a `>` terminal-command mode or `:` line-jump mode in v1; those land as the relevant host modules register their own command entries.
