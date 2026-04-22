# Session Roster & Asset Card

> **Status: DEFERRED post-Phase-2.** No component exists; no sidebar hosts this in the current session shell. The Phase 2 session shell is ActivityBar + FilesSidebar (file tree only) — there is no sidebar surface for a roster. Parked until Phase 3+ when asset loading becomes user-visible. Retained as forward-looking reference; do not cite as a paired spec.

> Extracted from SPECS.md §12-13 — scope-aware asset display and the repeatable asset card for skills, MCP servers, and agents

---

## 12. Session roster

**Purpose.** Show at a glance what's loaded into the current session — providers, fallback, agents, skills, MCPs, sandbox — with each entry's scope.

**Where.** Bottom of the session sidebar, below the file tree.

**Size.** 230px wide, content-driven height.

### 12.1 Structure

```
SESSION ROSTER
─ session-wide ──────
● provider     sonnet-4.5
🧩 skill       typescript-review
≡ mcp          github
📦 sandbox     process

─ agent: refactor-bot ─
🧩 skill       postgres-schemata

─ provider: anthropic ─
🧩 skill       long-context-helper
```

- Sections grouped by scope (session-wide / agent / provider)
- Each row: icon (14px), label (text-secondary 11px), value (text-primary Fira Code 10px, right-aligned)
- Section header: Fira Code 9px uppercase, letter-spacing 0.2em, text-tertiary

### 12.2 Interaction

- Hover a row: background `--color-surface-3`
- Click a row: jumps to the corresponding catalog view filtered to this asset
- Click a section header: collapses the section
- Per-section add button: `+` opens the catalog filtered by asset type with "Add to this scope" pre-selected

### 12.3 Session state variants

- **No agent session** (started from `forge run provider ...`): shows `● provider` line only, no agent header
- **Agent session**: shows all scoped assets grouped as above
- **Background agents active**: an additional `─ background ─` section lists each background agent

**Doesn't do.**
- Does not let you swap providers inline — use the composer selector
- Does not show usage — that's in pane headers and the usage view

---

## 13. Asset card (skill / MCP / agent)

**Purpose.** A single repeatable card used in the catalog view to represent any installable/enableable asset.

**Size.** min-width 340px, height content-driven (typically ~140–180px).

**Structure.**
```
┌────────────────────────────────────────────┐
│ [SIGIL] name                    [toggle]  │ ← a-head
│         scope · transport · version        │
├────────────────────────────────────────────┤
│ description — 1–3 lines, secondary color   │
├────────────────────────────────────────────┤
│ [tag] [tag] [tag]                health   │ ← foot
└────────────────────────────────────────────┘
```

**Sigil.** 34×34, `--color-bg` background, 1px border `--color-border-1`, radius `--r-sm`. Shows either:
- 2-letter initial in Barlow Condensed 900 14px, colored by asset type:
  - Skill: ember-400
  - MCP: ember-400 (default), steel (local), info (fetch/url), err/warn (tinted by state)
  - Agent: ember-400 (user-space), amber (openai-hosted), etc. per provider

**Name row.** Asset name + optional live-dot (ok/warn/err) when the asset has live connectivity.

**Scope line.** `workspace`|`user` · `stdio`|`http`|`container` · `v<version>` · source (URL or path).

**Portability hint.** Assets following open standards (agentskills.io skills, universal-standard MCP servers, `.agents/*.md` agents) show a small `portable` pill in the foot row — signals that this asset file works in other tools, not just Forge.

**Tags.** Small pills, Fira Code 10px. Common: `N tools`, `oauth`, `api key`, `local`, `scoped`, `container`, `readonly`, `rate-limited`.

**Health.** Right-aligned on the foot row: `<ms> p50`, `reconnecting · Ns`, `401 · reauth`, `disabled`, `—`.

**States.**
- enabled: toggle on (ember), card normal
- disabled: toggle off, card at 100% color (no dim)
- error: live-dot err, health shows error
- connecting: live-dot warn, health shows status

**Doesn't do.**
- Does not show per-asset logs inline — click card opens a detail drawer
- Does not permit editing in-place — detail drawer has the editor
