# Tool Call Card

> Extracted from SPECS.md §5 — inline expandable card, collapsed/expanded states, approval gating, parallel grouping, and nested sub-cards

---

## 5. Tool call card

**Purpose.** Surface every tool invocation inline in the chat, with full transparency — name, arguments, result, duration, and approval state.

**Size.** Full width of message, content-driven height.

**Collapsed state (default for prior calls, expanded for the latest).**
```
[🔧] fs.read  path: src/payments/processor.ts      42ms ✓  ›
```
- Icon: `--color-ember-100` (gold) for general tools, `--color-info` (steel) for pure reads, `--color-ember-400` (ember) for agent spawns
- Name in Fira Code 11px, color matches icon
- Arg summary in text-secondary, truncated with ellipsis
- Duration in text-tertiary mono 11px
- Status: `✓` ok, `!` warning/pending, `✗` error
- Chevron rotates 90° when expanded

**Expanded state.**
- Background: `rgba(255,209,102,0.04)`, border `rgba(255,209,102,0.15)`, radius `--r-sm`
- Body shows full arg JSON (pretty-printed), result preview (truncated at 800 chars with "show more"), and metadata: sha of file, bytes, lines, exit code, etc.
- For destructive calls (write, exec), expanded body shows a **diff preview** or **command preview** and the approval state

**Approval gating.** Writes and exec calls require approval (per CONCEPT.md §6.3). While awaiting approval: duration replaced with `awaiting approval`, status icon is `!` in warn color, body is expanded by default, approval UI (§10) renders within the body.

### 5.1 Parallel tool call grouping

When the model issues multiple read-only tool calls in the same turn, Forge executes them in parallel and visually groups them under a single expandable card:

```
[🔧] parallel reads · 3 calls                       48ms ✓  ›
  ├ fs.read   src/a.ts        14ms ✓
  ├ fs.read   src/b.ts         9ms ✓
  └ fs.read   src/c.ts        25ms ✓
```
- Summary row shows aggregate duration (max across parallel children)
- Expanded view shows each child card, each independently collapsible
- Writes never appear here — they always render as individual cards with individual approvals

**Sub-cards.** A tool call may contain nested calls (e.g. an MCP proxy that invokes other tools). Nested cards indent `--sp-5` and draw a 2px left border in `--color-border-1`.

**Doesn't do.**
- Never collapses the *currently streaming* tool call
- Never hides arguments for security review — truncation only
