# Sub-Agent Banner

> Extracted from SPECS.md §6 — orchestrator visualization, collapsed/expanded sub-thread, interaction model

---

## 6. Sub-agent banner

**Purpose.** Make it obvious when an orchestrator spawns a child agent, and provide a collapsible view of the child's thread inline.

**Size.** Full width, content-driven.

**Visual anatomy.**
```
┌───────────────────────────────────────────────────────────────────────────┐
│[↳] ↳ spawned · test-writer   delegated at 14:37   [sonnet-4.5]  [4 tools]  [running]  [>]
├───────────────────────────────────────────────────────────────────────────┤
│  sub-agent body: message thread, indented,                                │
│  with its own tool calls nested inside                                    │
└───────────────────────────────────────────────────────────────────────────┘
```
- Container: `--color-surface-1` bg, 1px `--color-border-1`, **2px left border `--color-ember-400`**, radius `--r-sm`
- Header (left → right): `↳ spawned · <n>` in Fira Code 11px text-primary, then `delegated at HH:MM` in text-tertiary 10px, then the optional `[model]` and `[N tools]` chips, then the state chip pushed right, then the chevron
- `[model]` and `[N tools]` chips appear only when the orchestrator forwards the corresponding fields on `SubAgentBannerTurn`. Each hides independently when absent
- State chip values: `running` / `done` / `queued` / `error` / `killed`
- State chip uses semantic color: running=warn, done=ok, queued=text-secondary, error=err, killed=text-tertiary

**Collapsed state.**
- Just the header row. Chevron on far right opens.
- Body replaced with a 1-line summary: `6 of 11 steps · last: wrote validate.test.ts`

**Expanded state.**
- Full indented sub-thread. Sub-agent's own tool calls render as normal tool-call cards inside.
- Max depth: 3. Beyond that, child banners render collapsed by default and "Open in new window" appears.

**Interaction.**
- Click header: toggle collapse
- Double-click header: focus the agent in the Agent Monitor (§9)
- Click state chip: open the sub-agent status details popover. The chip is a `<button>` whose click does **not** toggle the header collapse (event is stopped at the chip). The popover surfaces child instance id, status, started-at, last-step summary, and an "Open in Agent Monitor" affordance. It dismisses on outside click or Escape

**Doesn't do.**
- Does not let the user inject messages directly into a sub-agent's thread from here. To steer a sub-agent, use Agent Monitor's "interrupt + refine".
