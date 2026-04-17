# Approval Prompt

> Extracted from SPECS.md §10 — inline four-scope whitelisting, keyboard shortcuts, preview by tool type, whitelist indicators, and timeout

---

## 10. Approval prompt

**Purpose.** Get explicit human approval for tool calls that write, execute, or reach the network.

**Where it renders.** Inline, inside the tool-call card's expanded body. **Never a modal.** Modals halt the user; inline keeps the conversation flowing.

**Layout.**
```
┌───────────────────────────────────────────┐
│ fs.edit  processor.ts · 3 hunks · +47 -21 │  ← header (warn color)
├───────────────────────────────────────────┤
│ [diff preview — up to 20 lines per hunk]  │
│                                           │
│ --- before                                │
│ +++ after                                 │
│ @@ ...                                    │
├───────────────────────────────────────────┤
│ [Reject]            [Approve ▾]           │
│                      └ Once               │
│                        This file          │
│                        This pattern       │
│                        This tool          │
└───────────────────────────────────────────┘
```

### 10.1 Approval buttons

**Reject** — ghost, text-secondary. Cancels the call; agent receives a rejection message.

**Approve ▾** — primary, ember. Click opens a menu with four scopes:

| Scope | Meaning |
|---|---|
| **Once** | Approve this exact call only. Next write prompts again. |
| **This file** | Approve this tool for this specific file/path for the rest of the session. |
| **This pattern** | Approve this tool for a glob that matches this path (e.g. `./src/*`). User can edit the pattern before confirming. |
| **This tool** | Approve this tool type entirely for the session (e.g. all `fs.write`). |

All whitelist scopes (file/pattern/tool) are **session-local** — they reset when the session ends. No persistent whitelists.

### 10.2 Keyboard

- `R` — Reject
- `A` — Approve once (default; also `Enter`)
- `F` — Approve this file
- `P` — Approve this pattern (opens a small editor for the glob)
- `T` — Approve this tool

### 10.3 Preview by tool type

Non-destructive previews render based on the tool:
- `fs.edit`: diff view (up to 20 lines per hunk shown inline, "show full diff" link if more)
- `fs.write`: path + bytes + language-tinted preview (first 40 lines)
- `shell.exec`: cmd in mono 12px, argv pretty-printed, cwd shown, env vars in a collapsible section
- Network tool calls: destination host/port, method, headers (auth redacted), body preview

### 10.4 Whitelist indicators

After a scope approval is granted, subsequent matching calls show a small green `whitelisted · this file` / `whitelisted · pattern ./src/*` / `whitelisted · tool` pill in the tool-call card header instead of the approval UI. Clicking the pill opens a popover with `Revoke for this session` option.

### 10.5 Timeout

- Default: never. Approval waits indefinitely, agent is paused.
- Configurable per-agent to auto-reject after N seconds (for headless CI use via `approval_timeout_sec: 30` in frontmatter).

**Doesn't do.**
- Does not have a global "always approve this tool" setting. Whitelist scope is session only.
- Does not batch multiple pending approvals — each call is approved separately.
