# Sub-Agent Banner

> Extracted from SPECS.md §6 — orchestrator visualization, collapsed/expanded sub-thread, interaction model

---

## 6. Sub-agent banner

**Purpose.** Make it obvious when an orchestrator spawns a child agent, and provide a collapsible view of the child's thread inline.

**Size.** Full width, content-driven.

**Phase 2 scope.** The banner ships with glyph, agent name, delegated-at timestamp, state chip, and chevron. Model and tool-count chips, plus the state-chip details popover, are **deferred to Phase 3** (tracked in F-448) — they require wire fields the orchestrator does not forward today (`SubAgentBannerTurn` carries no `model` or `tool_count`) and a popover surface that Phase 2 does not host.

**Visual anatomy (Phase 2 — shipped).**
```
┌───────────────────────────────────────────────────┐
│[↳] ↳ spawned · test-writer   delegated at 14:37   [running]  [>]
├───────────────────────────────────────────────────┤
│  sub-agent body: message thread, indented,       │
│  with its own tool calls nested inside           │
└───────────────────────────────────────────────────┘
```
- Container: `--color-surface-1` bg, 1px `--color-border-1`, **2px left border `--color-ember-400`**, radius `--r-sm`
- Header (left → right): `↳ spawned · <n>` in Fira Code 11px text-primary, then `delegated at HH:MM` in text-tertiary 10px, then the state chip pushed right, then the chevron
- State chip values: `running` / `done` / `queued` / `error` / `killed`
- State chip uses semantic color: running=warn, done=ok, queued=text-secondary, error=err, killed=text-tertiary

**Visual anatomy (Phase 3 — deferred).** Once the orchestrator forwards `model` and `tool_count` on `SubAgentBannerTurn`, the header adds `[model]` and `[N tools]` chips between the timestamp and the state chip, and the state chip becomes an interactive `<button>` that opens the status-details popover (see Interaction). Header shape becomes `[↳] ↳ spawned · <n>  delegated at HH:MM  [sonnet-4.5] [4 tools] [running]  [>]`.

**Collapsed state.**
- Just the header row. Chevron on far right opens.
- Body replaced with a 1-line summary: `6 of 11 steps · last: wrote validate.test.ts`

**Expanded state.**
- Full indented sub-thread. Sub-agent's own tool calls render as normal tool-call cards inside.
- Max depth: 3. Beyond that, child banners render collapsed by default and "Open in new window" appears.

**Interaction.**
- Click header: toggle collapse
- Double-click header: focus the agent in the Agent Monitor (§9) *(Phase 2)*
- Click state chip: show sub-agent status details popover *(Phase 3 — deferred; state chip is a passive label in Phase 2)*

**Doesn't do.**
- Does not let the user inject messages directly into a sub-agent's thread from here. To steer a sub-agent, use Agent Monitor's "interrupt + refine".
