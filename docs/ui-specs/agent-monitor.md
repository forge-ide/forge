# Agent Monitor

> Extracted from SPECS.md §9 — three-column layout: agent list, trace timeline, and inspector panel

---

## 9. Agent monitor

**Purpose.** Observe, trace, and control every agent across all sessions from one view.

**Layout.** Three columns: list (280px) | trace (flex) | inspector (340px).

### 9.1 Agent list (left)

**Row.**
```
[provider dot] name                            #a3f1.1
              sub-agent · 6/11 steps
[============================            ]          ← progress bar, 3px
running · 3m · $0.09              sonnet-4.5        ← meta row
```
- Progress bar colors by state: done=ok, running=warn, queued=text-tertiary, error=err
- Active agent has 2px left border in ember and `--color-surface-2` bg
- Hover: `--color-surface-2`

**Background agent marker.** Agents that are user-initiated top-level background workers (not sub-agents of anything) show a small `BG` pill in the meta row. Sub-agents show `↳ parent-name` instead.

**Sort.** Default: running first, then queued, then error, then done — within each group, most recent first.

**Filter.** Tabs at the top: `All · Running · Background · Session · Failed`.

### 9.2 Trace (middle)

**Header.** Agent name big, id small, live chip (`running · step 6 of 11`), `Pause` / `Kill` buttons, `Promote to pane` button (for background agents).

**Toolbar.** Shows elapsed, token in/out, cost, model, tools-used, spawned-by relationship — all in Fira Code 10px separated by `·`.

**Timeline.** Vertical list of steps. Each step:
- 16px filled dot, colored by state (done/ok, run/warn, queued/text-tertiary, err/error)
- 2px vertical rail connecting dots
- Running step has a pulsing ring (2s infinite, opacity 0→1)
- Content: step kind chip (`tool`, `think`, `spawn`, `mcp`), title line, optional description, optional preview box (mono 11px)

**Step kinds and colors.**
| Kind | Chip bg | Chip text | Meaning |
|---|---|---|---|
| `tool` | `rgba(255,209,102,.05)` | ember-100 | Tool invocation (fs, shell, etc.) |
| `mcp` | `info-bg` | info | MCP server call |
| `spawn` | `rgba(255,74,18,.05)` | ember-400 | Child agent spawned |
| `think` | surface-2 | text-secondary | Model reasoning pass |

**Interaction.**
- Click step: expands preview inline
- Double-click: opens a detail drawer from the right

### 9.3 Inspector (right)

Five sections:

1. **Definition.** name, source (file + line), provider, model, isolation level, max tokens.
2. **Allowed tools.** Pills, each with click-to-view policy.
3. **Allowed paths.** Pills with mono text; glob patterns rendered verbatim.
4. **Resource usage.** cpu, rss, fd open, net connections — live, 1Hz update.
5. **Actions.** `Pause agent`, `Interrupt + refine` (opens a refine composer in context), `Export transcript` (JSONL), `Promote to pane` (background only).

**Doesn't do.**
- Does not let you edit agent definitions inline (opens the source file instead)
- Does not surface prompts verbatim — use `Export transcript` for the full record
