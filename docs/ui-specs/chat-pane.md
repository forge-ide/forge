# Chat Pane

> Extracted from SPECS.md §4 — message anatomy, composer, streaming behaviours, keyboard shortcuts, and narrow-width adaptations

---

## 4. Chat pane

**Purpose.** Conversation with an agent. The *only* surface where streaming happens.

**Size.** Fills its pane area. Minimum width 320px — below that, pane header labels collapse to icons.

**Header.**
- Type label: `CHAT`
- Subject: `<provider dot> <agent name> · <model>`
- Right actions: **cost meter** `in <n> · out <n> · $<cost>` in mono 10px, then overflow menu

**Body structure (top to bottom).**
```
[ ↑ scrollback ]
[ message: user ]
[ message: assistant ]
  [ tool-call card ] ← inline, expanded by default for latest
  [ sub-agent banner ] ← inline, collapsible sub-thread
[ message: user ]
[ message: assistant ]  ← current, streaming
  [ streaming cursor ]
[ ↓ composer ]
```

**Message anatomy.**
- Author strip: `● you` (text-primary) or `● <provider>` (provider accent), plus timestamp right-aligned
- Author label is 10px mono uppercase, letter-spacing 0.12em
- Body is 13.5px Barlow, line-height 1.55, text-primary
- Inline code uses 12px Fira Code, `--color-surface-2` bg, 1px `--color-border-1`, radius 3px
- Hovering a message reveals a small action row: `Copy`, `Re-run ▾`, `Branch...` (for user messages), `Edit` (deliberate no-op — transcripts are read-only; this is just a "copy and refine" shortcut that pre-fills the composer)

**Composer.** See §4.1.

**Spacing.**
- Between messages: `--sp-5` (20px)
- Between author strip and body: `--sp-2` (8px)
- Side padding: `--sp-5`

**Narrow-width adaptations.** When the pane is less than 480px wide:
- Tool-call card arg summaries hard-truncate at 1 line with ellipsis
- Composer ctx chip row can scroll horizontally instead of wrapping
- Cost meter in header collapses to `$<cost>` only; tokens appear in a tooltip

### 4.1 Composer

**Size.** Auto-sized to content, min-height ~94px collapsed, max 40% of pane height before scrolling.

**Structure.**
```
[ ctx chips row ]           ← @-references, removable
┌─────────────────────────┐
│ textarea                │ ← placeholder: "Ask, refine,
│                         │    or @-reference context"
│                         │
├─────────────────────────┤
│ [provider·model] @ctx / | [Stop] [Send ⌘↵]
└─────────────────────────┘
```

**Input wrap.** `--color-surface-2` bg, 1px `--color-border-1`, radius `--r-sm`. Focus border is `--color-ember-400`, always.

**Bottom bar.**
- Left: provider pill (clickable, opens selector §8), then `@ for context`, `/ for commands` ghost pills
- Right: `Stop` (ghost), `Send ⌘↵` (primary, ember)

**Keyboard.**
- `Enter`: newline
- `Cmd/Ctrl+Enter`: send
- `@`: opens context picker
- `/`: opens command palette filtered to session commands
- `Esc` while streaming: stop (same as Stop button)
- `↑`: cycles previous user messages into the textarea

**Streaming-time behaviours.**
- Send button becomes disabled with `Streaming…` label
- Stop button becomes primary (ember)
- Composer remains editable — pressing Send while streaming queues a follow-up *after* stop or completion (with a small queued-indicator)

**Compact button.** The cost meter area in the pane header has a small "compact" button that becomes active at 90%+ of the context window. Clicking it triggers the compaction flow (see CONCEPT.md §10.4). Icon is a downward-pointing chevron with text label `compact` at pane widths > 400px.

**Doesn't do.**
- Does not support image attachments in v1 (queued for v1.1; placeholder UX exists)
- Does not render markdown in the textarea itself — input is plain text until sent
