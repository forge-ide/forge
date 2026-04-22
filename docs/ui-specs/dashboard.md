# Dashboard

> Root window of the desktop app — surfaces the active provider and the session roster so the user can pick up where they left off or start something new.

---

## D. Dashboard

**Purpose.** First view in the Dashboard window. Confirms the local environment is wired up (provider reachable, models discoverable) and lets the user re-enter an existing session or read what's been archived.

**Where.** Root route (`/`) of the Dashboard window. There is exactly one Dashboard window per app instance.

**Size.** Fills the window. Min-height `100vh`. Single-column layout, content-driven height.

### D.1 Structure

```
┌───────────────────────────────────────────────┐
│ Forge — Dashboard                             │ ← title, ember
├───────────────────────────────────────────────┤
│ ┌── PROVIDER · ollama ────────────────────┐   │
│ │ ● http://localhost:11434                 │   │
│ │ [SHOW MODELS — 4 models]   [REFRESH]     │   │
│ └─────────────────────────────────────────┘   │
├───────────────────────────────────────────────┤
│ [active 03] [archived 12]                     │
│ ┌─────────────┐ ┌─────────────┐ ┌──────────┐  │
│ │ session     │ │ session     │ │ session  │  │
│ │ specimen    │ │ specimen    │ │ specimen │  │
│ └─────────────┘ └─────────────┘ └──────────┘  │
└───────────────────────────────────────────────┘
```

Top-to-bottom: title → provider panel → sessions panel. No tab bar, no sidebar, no pane splits — the Dashboard is intentionally a single flat surface.

### D.2 Title

- Text: `Forge — Dashboard` (em dash, single space either side)
- Font: `--font-display` (Barlow Condensed)
- Size: 2rem
- Color: `--color-text-ember`
- Margin: `0 0 var(--sp-3) 0`

### D.3 Spacing & background

- Page padding: `var(--sp-8)` on all sides
- Background: `--color-bg`
- Body color: `--color-text-primary`
- Body font: `--font-body`

### D.4 Composition

The Dashboard renders three children in order: `<h1>` title, `<ProviderPanel/>`, `<SessionsPanel/>`. Both panels self-source their data via Tauri commands (`provider_status`, `session_list`) and own their own loading and empty states.

- **Provider panel** — see `web/packages/app/src/routes/Dashboard/ProviderPanel.tsx`. Shows the configured provider (Phase 1: ollama, hardcoded), reachability indicator, base URL, expandable model list, and a `REFRESH` action that re-probes. When unreachable, renders an `ECONNREFUSED <host>` line and a `START OLLAMA` CTA. Provider identity color follows `ai-patterns.md` — Ollama is `steel`.
- **Sessions panel** — industrial-ledger specimen cards grouped by `active` / `archived` tabs. Card click dispatches `open_session` and reopens the Session window. Cards show subject, persistence badge, state pip, provider chip, and a relative `last event` timestamp.

### D.5 States

The Dashboard itself has no global loading or error state — it always renders the title and both panels. The panels each handle their own state per `component-principles.md`'s four-state rule (loading / error / empty / ready):

- **Provider panel — loading:** placeholder line `ollama · probing` (noun + state per `voice-terminology.md` §8) while `provider_status` resolves.
- **Provider panel — unreachable:** error line + remediation hint + `START OLLAMA` CTA.
- **Sessions panel — loading:** placeholder line `sessions · probing` while `session_list` resolves.
- **Sessions panel — empty (active tab):** `// no active sessions`.
- **Sessions panel — empty (archived tab):** `// archive is empty`.
- **Sessions panel — fetch failure:** visible error block with heading `SESSIONS UNAVAILABLE`, the verbatim error detail (preserved exactly per `voice-terminology.md` §8 "show technical identifiers verbatim"), and a `RETRY` button that re-invokes `session_list`. The error state is distinct from empty — the `session_list` rejection must not collapse to `// no active sessions`.

### D.6 Cross-spec references

- `session-roster.md` *(deferred post-Phase-2)* — forward-looking spec for an in-session roster of loaded assets. No component or hosting sidebar exists in Phase 2; the Dashboard's session cards still open sessions, but those sessions do not render a roster today.
- `ai-patterns.md` — provider accent colors used by the provider indicator and any future provider chips on session cards.
- `provider-selector.md` — composer-time provider switching (Phase 2+). The Dashboard's provider panel is read-only status, not a switcher.
- `layout-panes.md` — pane model used by the Session window. The Dashboard does not use the pane model.

**Doesn't do.**
- Does not start a new session inline — `forge run …` from a terminal still seeds new sessions in Phase 1; a `NEW SESSION` action arrives later.
- Does not show usage, billing, or per-model cost — that surfaces in pane headers and the usage view.
- Does not configure providers — provider config lives in `~/.config/forge/providers.toml`; the panel only reflects status.
