# Message Tree Branching

> Extracted from SPECS.md §15 — message tree data model, rendering, conversation flow, branch metadata card, event log implications, and v1 scope

---

## 15. Message tree branching

**Purpose.** Support the "Branch" variant of re-run (CONCEPT.md §10.3) — the user asks the model to redo a response with a different provider or refined wording, while keeping the original answer intact for comparison.

### 15.1 Data model

A message has an optional `branch_parent_id` and `branch_variant_index`. When a user triggers Branch re-run on message M:
1. Forge appends a new `AssistantMessage` event with `branch_parent_id = M.branch_parent_id ?? M.id` and `branch_variant_index = (max existing variant index at that branch point) + 1`
2. Both messages remain in the event log; the log is still append-only
3. One variant is marked as the *active* branch via a `BranchSelected` event; the others are inactive but preserved

### 15.2 Rendering — single-variant messages
Messages with no branches render identically to today's chat pane — no extra chrome.

### 15.3 Rendering — branched messages

When an assistant message has branches, a small variant selector appears as the first line of the message body, before the author strip:

```
┌────────────────────────────────────────────┐
│ ◀  variant 2 of 3  ▶     [branch info ⓘ]  │ ← 20px strip, surface-2 bg
│ ● claude · sonnet-4.5           14:24:35   │
│ Understood. Re-drafting the edit…          │
└────────────────────────────────────────────┘
```

- Left/right arrows cycle through variants
- Middle label shows `variant N of M`
- Info icon opens a popover: provider/model of each variant, timestamp, first-line preview, click-to-select
- Current variant is the one rendered; switching is instant (no animation)

### 15.4 Conversation flow after a branch

Continuing the conversation from a branched message:
- Typing a new user message while on variant 2 creates the user message as a child of variant 2
- If the user switches to variant 3 later, they see variant 3's children (which may be different)
- Each variant has its own downstream tree

Visually, when scrolling past a branched message, a subtle indicator in the gutter shows which variant path the conversation is currently on: a 2px vertical line in `--color-ember-300` that threads between the variant selector and its children.

### 15.5 Branch metadata card

Clicking the info icon opens a popover with a summary:

```
┌─────────────────────────────────────┐
│ 3 variants of this response         │
├─────────────────────────────────────┤
│ ● variant 1   sonnet-4.5  14:22:11  │
│   "I'll read the current…"          │
│                                     │
│ ● variant 2   sonnet-4.5  14:24:35  │
│   "Understood. Re-drafting…" (active)│
│                                     │
│ ● variant 3   opus-4.1    14:26:02  │
│   "Let me take a different…"        │
├─────────────────────────────────────┤
│ [Delete variant 1]  [Export all]    │
└─────────────────────────────────────┘
```

### 15.6 Event log implications

- Event log replay must reconstruct the branch tree correctly
- `BranchSelected` events let replay know which variant was active
- If a session ends mid-branch-edit, reopening restores the last-selected variant
- `forge session export` can export all branches (default) or just the active path (`--active-only`)

### 15.7 Interactions with tool calls

Tool calls that happened inside variant 2 stay with variant 2. Switching to variant 3 hides variant 2's tool calls and shows variant 3's. This can be disorienting; a small `switched to variant N` toast appears on switch.

### 15.8 Out of scope for v1

- Merging branches (taking content from two variants and combining)
- Named branches (variants are numeric only)
- Branch diff view (comparing two variants side by side as a dedicated surface)

These are v1.1+ ideas; the minimal branching above gives us the shape to build on.
