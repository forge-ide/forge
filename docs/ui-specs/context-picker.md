# @-Context Picker

> Extracted from SPECS.md §7 — @-context picker trigger, 360-480px popup, categories, keyboard, chip insertion, and sizing rules

---

## 7. @-context picker

**Purpose.** Universal way to attach any reference (file, selection, terminal output, another agent) into the current message.

**Trigger.** Typing `@` in any chat composer.

**Size.** 360–480px wide, up to 360px tall (scrolls).

**Placement.** Viewport-aware and computed by the pure `computePopupPlacement` helper in `web/packages/app/src/components/ContextPicker.tsx`. The popup prefers anchoring *below* the caret/composer when there is room — specifically, when `viewportHeight − anchorBottom − gap ≥ popupHeight` (gap defaults to 4px, popup height is capped at 360px). When that space is not available, placement flips to *above* the anchor so the popup is never clipped by the viewport bottom. In the standard chat layout the composer is pinned to the bottom of its pane, so the flip-above branch is the one that renders in practice; the below branch is what keeps the popup from being clipped when the composer is embedded in pickers or other non-bottom-pinned hosts. Either way the composer itself remains visible.

**Structure.** Three stacked rows: an `@`-echo search row, a horizontal strip of seven category tabs, then the results list for the active category. Only the active category's results render; switching tabs swaps the list.

```
┌──────────────────────────────────────────┐
│ @ src/foo                                │  ← search row echoes the live @-query
├──────────────────────────────────────────┤
│ [F] FILE  [D] DIR  [S] SEL  [T] TERM …   │  ← tablist; active tab is ember-bordered
├──────────────────────────────────────────┤
│ ▸ client.ts                              │  ← results for the active category only
│   processor.ts                           │     (or "No <category> results" when empty)
│   payments/handler.ts                    │
└──────────────────────────────────────────┘
```

The tab strip renders all seven categories in order: `file`, `directory`, `selection`, `terminal`, `agent`, `skill`, `url`. Each tab shows a bracketed glyph (`[F]`, `[D]`, `[S]`, `[T]`, `[A]`, `[K]`, `[U]`) plus a lowercase mono label that is rendered uppercase via CSS. The active tab is highlighted with an ember border and elevated surface; the active result row carries an ember left border.

**Categories.**
- `file` — any file in workspace. Uses `.gitignore`.
- `directory` — inserts as tree snapshot (paths only, respecting gitignore)
- `selection` — active editor selection, if any; otherwise absent
- `terminal` — last N lines of the focused terminal pane
- `agent` — another agent's transcript (inserted as summary + inline references, not full copy)
- `skill` — a skill definition (reference, not content — the agent will load it)
- `url` — paste or type a URL; Forge fetches at send time (respecting allowed hosts)

**Interaction.** DOM focus stays in the composer textarea while the popup is open; the combobox root carries `aria-activedescendant` pointing at the active result row (WAI-ARIA combobox pattern).

- Up / Down: move the cursor within the active category's results (wraps at ends).
- Tab: cycle to the next category. Shift+Tab cycles to the previous category. Both wrap at the ends. The result cursor resets to the first row of the newly-active category.
- Enter: insert the active result as a chip and close the picker. The `@`-token in the textarea is replaced by the chip.
- Esc: close the picker. Any typed `@`-text is retained in the textarea.
- Mouse: clicking a tab activates its category; clicking a result inserts that result as a chip. Both use `mousedown` with `preventDefault` so the composer never loses focus.

> **Note on "insert and refine".** An earlier draft of this spec defined Tab as "insert and refine" — inserting the current candidate while keeping the picker open for follow-up refinement. That behavior is deferred (tracked as a possible v1.1 enhancement) and is not a current requirement. Tab currently switches categories.

**On insert.**
- A chip appears in the ctx-chips row above the textarea: `[F] processor.ts ×` (icon glyph matches the picker tab: `[F]`, `[D]`, `[S]`, `[T]`, `[A]`, `[K]`, `[U]`)
- The `@text` is removed from the textarea
- At send, each chip resolves to an appropriate context block per-provider (XML tags for Anthropic, function-style for OpenAI, etc.)

**Sizing rules.**
- File previews are lazy — list shows path + 1 line of content; hover expands
- Directory contexts are capped at 200 entries in v1; show `+N more` if truncated

**Doesn't do.**
- Does not support @-mentioning yourself or the session
- Does not let the user inline-edit a referenced file's content from the picker
