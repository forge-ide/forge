# @-Context Picker

> Extracted from SPECS.md §7 — @-context picker trigger, 360-480px popup, categories, keyboard, chip insertion, and sizing rules

---

## 7. @-context picker

**Purpose.** Universal way to attach any reference (file, selection, terminal output, another agent) into the current message.

**Trigger.** Typing `@` in any chat composer.

**Size.** 360–480px wide, up to 360px tall (scrolls), popped *above* the composer (never below — the composer must remain visible).

**Structure.**
```
┌──────────────────────────────────────────┐
│ SEARCH…              │ Cmd+K             │
├──────────────────────────────────────────┤
│ CATEGORY             │ (recent)          │
│ [📄] file            client.ts           │
│ [📄] file            processor.ts        │
│ [📁] directory       tests/payments      │
│ [↳]  selection       editor @ ln 14-22   │
│ [≡]  terminal        last 20 lines       │
│ [🤖] agent           refactor-bot thread │
│ [🧩] skill           typescript-review   │
│ [🌐] url             https://...          │
└──────────────────────────────────────────┘
```

**Categories.**
- `file` — any file in workspace. Uses `.gitignore`.
- `directory` — inserts as tree snapshot (paths only, respecting gitignore)
- `selection` — active editor selection, if any; otherwise absent
- `terminal` — last N lines of the focused terminal pane
- `agent` — another agent's transcript (inserted as summary + inline references, not full copy)
- `skill` — a skill definition (reference, not content — the agent will load it)
- `url` — paste or type a URL; Forge fetches at send time (respecting allowed hosts)

**Interaction.**
- Up/down: navigate
- Enter: insert as chip
- Tab: insert and refine (replaces last token, cursor remains in picker)
- Esc: close, retain typed text

**On insert.**
- A chip appears in the ctx-chips row above the textarea: `[📄 processor.ts ×]`
- The `@text` is removed from the textarea
- At send, each chip resolves to an appropriate context block per-provider (XML tags for Anthropic, function-style for OpenAI, etc.)

**Sizing rules.**
- File previews are lazy — list shows path + 1 line of content; hover expands
- Directory contexts are capped at 200 entries in v1; show `+N more` if truncated

**Doesn't do.**
- Does not support @-mentioning yourself or the session
- Does not let the user inline-edit a referenced file's content from the picker
