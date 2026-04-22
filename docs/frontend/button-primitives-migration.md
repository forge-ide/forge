# Button Primitives Migration Plan

> Phase-3 migration plan for extracting `Button`, `IconButton`, `Tab+Tabs`, and
> `MenuItem` into `@forge/design` and replacing the 44 raw `<button>` sites
> catalogued in F-398. Planning-only artifact — no code change in this phase.

**Status:** planning (F-398). Implementation tracked by the Phase-3 sibling
issue named in F-398's DoD.

**Related docs:** [Component Principles](../design/component-principles.md) ·
[Voice & Terminology](../design/voice-terminology.md) ·
[Generation Pipelines](generation-pipelines.md)

---

## Why

`@forge/design` ships `tokens.css` today. Every button in the app re-implements
`type="button"`, focus-visible outline, hover state, uppercase typography, and
a11y wiring — 44 sites across 16 files. That is consistency-drift risk, not a
correctness bug, so the extraction is deferred to Phase 3 per F-398. This doc
pins the target shape so a Phase-3 PR set can land mechanically.

## Non-goals

- **No runtime behavior change.** Primitives must render byte-equivalent DOM
  (plus `aria-*` additions where currently missing) so visual regression is
  zero.
- **No new design decisions.** The primitive surface mirrors the variants
  already documented in [Component Principles §Buttons](../design/component-principles.md#buttons).
  If a site's current styling drifts from the design doc, migration fixes the
  site — not the primitive.
- **No tokenization of row/card-as-button patterns** (see allowlist below).
  Those are content-shaped; wrapping them in a primitive would leak layout
  concerns into the primitive's API.

---

## Primitives

All primitives live in `web/packages/design/src/button/` and export through
`web/packages/design/src/index.ts`. Solid components, props typed with
`ComponentProps<'button'>` extension so `onClick`, `disabled`, `aria-*`, etc.
pass through. Source label strings must be literal UPPERCASE so screen readers
announce the casing correctly (per [Voice & Terminology](../design/voice-terminology.md#formatting-rules-for-ui-copy)).

### `Button` — 16 sites

The primary/secondary/ghost/danger archetype. Verb-noun display-caps label
lives as the component's children.

```tsx
type ButtonProps = ComponentProps<'button'> & {
  variant?: 'primary' | 'ghost' | 'danger';  // default: 'primary'
  size?: 'sm' | 'md';                        // default: 'md'
  leadingIcon?: JSX.Element;
  trailingIcon?: JSX.Element;
  loading?: boolean;                         // renders a spinner, disables click
  kbd?: string;                              // right-aligned keyboard hint
};
```

**Behavioral contract**
- `type="button"` is baked in — children never pass `type="submit"` accidentally.
- `disabled` renders the `iron-600` background per [Component Principles](../design/component-principles.md#buttons),
  never `opacity`.
- `loading` implies `disabled` and renders `aria-busy="true"`.
- Focus ring uses `--color-ember-400` outline; never suppressed.

**Migration sites (16):** #1, #2, #3, #17, #18 (button only; row stays raw),
#20, #21, #22, #24, #25, #28, #29, #30, #31, #34, #36, #37, #38, #39.
Exact line numbers in F-398 catalog.

### `IconButton` — 8 sites

Icon-only trigger. `label` is required so every IconButton has an accessible
name — this is the primary lint-rule payoff.

```tsx
type IconButtonProps = Omit<ComponentProps<'button'>, 'aria-label' | 'children'> & {
  icon: JSX.Element;
  label: string;                             // required → aria-label + title
  variant?: 'ghost' | 'danger';              // default: 'ghost'
  size?: 'sm' | 'md';                        // default: 'md'
  pressed?: boolean;                         // toggle state → aria-pressed
};
```

**Behavioral contract**
- `label` is passed through as both `aria-label` and `title`. No string-free
  escape hatch — the compile error is the point.
- `pressed` renders `aria-pressed` (toggle affordance for activity-bar icons).
- Focus ring identical to `Button` — inherited from `.forge-button--focus` class.

**Migration sites (8):** #4, #8, #9, #10, #11, #12, #23, #32, #40.

### `Tab` + `Tabs` — 7 sites

Roving-tabindex tab list. `Tabs` is the container (owns focus management);
`Tab` is the individual row. `variant="radio"` covers the approval-scope
pill pattern that currently abuses `role="radio"` on a button.

```tsx
type TabsProps = ComponentProps<'div'> & {
  activeId: string;
  onSelect: (id: string) => void;
  variant?: 'tab' | 'radio';                 // default: 'tab'
};

type TabProps = ComponentProps<'button'> & {
  id: string;
  label: string;
  badgeCount?: number;
};
```

**Behavioral contract**
- `<Tabs variant="tab">` renders `role="tablist"`; each `<Tab>` renders
  `role="tab"` + `aria-selected`.
- `<Tabs variant="radio">` renders `role="radiogroup"`; each `<Tab>` renders
  `role="radio"` + `aria-checked`.
- Arrow-key navigation inside `Tabs` moves focus between enabled children
  (roving tabindex — one child is tabbable, the rest are `tabindex={-1}`).
- Enter / Space on a focused tab fires `onSelect`.

**Migration sites (7):** #13, #17, #26, #33, #34, #35 (radio variant), and
one of the existing `role="tab"` call sites in `AgentMonitor.tsx` filter bar.

### `MenuItem` — 9 sites

Row in a `role="menu"` container. Does not own the menu container itself
(that stays per-feature — `FilesSidebar` context menu, `ApprovalPrompt`
scope menu, etc.); just normalises each row.

```tsx
type MenuItemProps = ComponentProps<'button'> & {
  label: string;
  leadingText?: string;                      // left-rail hint (e.g. file path scope)
  kbd?: string;                              // right-aligned keyboard hint
  variant?: 'default' | 'danger';            // default: 'default'
};
```

**Behavioral contract**
- `role="menuitem"` baked in — parent `<ul role="menu">` still owned by
  the feature.
- Arrow-key navigation is the parent menu's responsibility, not the primitive's.
- `danger` variant uses `--color-error` text; no separate border.
- Source label must be literal UPPERCASE (voice rule).

**Migration sites (9):** #5, #6, #7, #15, #16, #41, #42, #43, #44.

---

## Allowlist (row/card-as-button sites — stay raw)

Five files render a raw `<button>` where the entire content is a row or card
with layout concerns the primitive can't express cleanly. These are
allowlisted by `scripts/check-raw-buttons.mjs` and stay raw after migration:

| Catalog # | File | Why raw |
|---|---|---|
| 1 | `web/packages/app/src/shell/StatusBar.tsx` | bg-agents badge row — content-shape varies with count |
| 14 | `web/packages/app/src/components/BranchMetadataPopover.tsx` | variant row with chip + timestamp |
| 18, 19 | `web/packages/app/src/routes/AgentMonitor.tsx` | agent row / trace step — tree affordance |
| 27 | `web/packages/app/src/routes/Dashboard/SessionsPanel.tsx` | session card |

The _inner_ controls inside these rows (e.g. the stop-button at #3 inside the
StatusBar row, the delete button at #15 inside BranchMetadataPopover, the
tabs at #17 above AgentMonitor's agent row) still migrate to primitives.
Migration diff must keep the outer `<button>` and only rewrite the inner
controls.

---

## Migration PRs (4, grouped by archetype)

Each PR opens, lands, and ships independently. Hard sequencing: PR 1 must
merge before PR 2–4 because 2–4 depend on the primitives existing.

### PR 1 — Primitives + Button migration (16 sites)

- **Scope:** Add `Button`, `IconButton`, `Tab+Tabs`, `MenuItem` under
  `web/packages/design/src/button/`. Export through `@forge/design`.
  Migrate only the 16 `Button` sites; leave the rest raw for now.
- **Files:** 1 new directory in `web/packages/design/`, 16 files in
  `web/packages/app/src/` touched.
- **Tests:** Solid-testing-library unit tests per primitive (props → DOM
  shape, focus behavior, aria attrs). Visual parity checked with `@forge/design`
  Storybook-equivalent smoke route (optional).
- **CI:** green via existing `just check-web`. Raw-button lint stays
  disabled — the remaining archetypes still have raw buttons.

### PR 2 — IconButton migration (8 sites)

- **Scope:** Replace 8 icon-only raw buttons with `IconButton`. No primitive
  changes.
- **Files:** 8 TSX files touched — `ActivityBar`, `BranchSelectorStrip`
  (×3), `ContextChip`, `FilesSidebar` (refresh), `EditorPane` (close),
  `WhitelistedPill`, `ApprovalPrompt` (dropdown toggle).
- **Tests:** per-site snapshot check that the `aria-label` is present and
  matches the previous `aria-label` verbatim.

### PR 3 — Tab + MenuItem migration (16 sites)

- **Scope:** Replace 7 `Tab` call sites (including approval-scope
  `role="radio"` abusers via `<Tabs variant="radio">`) and 9 `MenuItem`
  rows.
- **Files:** `ContextPicker`, `AgentMonitor` (filter bar), `SessionsPanel`,
  `ApprovalPrompt`, `FilesSidebar` (context menu), `BranchMetadataPopover`
  (inner menu).
- **Tests:** keyboard-navigation tests for `Tabs` roving focus and
  menu-open → arrow-key stepping.

### PR 4 — Activate the raw-button lint (cleanup)

- **Scope:**
  1. Shrink `ALLOWLIST` in `scripts/check-raw-buttons.mjs` to only the five
     row/card files above.
  2. Add `node scripts/check-raw-buttons.mjs` to `justfile`'s `check-web`
     recipe, after `pnpm check-tokens`.
  3. Add `"check-raw-buttons": "node ../scripts/check-raw-buttons.mjs"` to
     `web/package.json`.
  4. Update this doc's header to mark planning as shipped.
- **Tests:** CI now fails on any new raw button. Nothing else changes.

---

## A11y requirements (cross-primitive)

- **Focus ring never suppressed.** All primitives use
  `outline: 2px solid var(--color-ember-400)` via a shared `.forge-button`
  class; no `:focus-visible { outline: none }` without a replacement.
- **Source-text uppercase, not CSS.** `text-transform: uppercase` remains for
  letter-spacing cosmetics, but the source text must already be uppercase so
  screen readers announce the intended casing (voice rule §8).
- **Disabled is semantic.** `disabled` maps to HTML `disabled` + token-driven
  `iron-600` background. No pointer-events / opacity escape hatch.
- **Icon-only always has an accessible name.** `IconButton.label` is required
  at the type level — this is the single biggest gap the primitives close.
- **Pressed / selected state.** `pressed`, `aria-checked`, `aria-selected`,
  `aria-expanded` wired where appropriate; no site should be reaching for
  `aria-*` directly through the primitive's `...rest` spread.

## Testing strategy

- **Unit (primitive):** Vitest + `@solidjs/testing-library` per primitive.
  Props → DOM shape, focus trap where relevant, keyboard interaction.
- **Unit (lint):** `scripts/check-raw-buttons.test.mjs` — fixture-driven, runs
  via `node scripts/check-raw-buttons.test.mjs`. See file header for how it
  drives `scanTsxSources`.
- **Integration:** existing Tauri webview smoke tests keep running; no new
  suites needed because DOM shape is parity-preserving.

---

## The raw-button lint (drafted, disabled)

Drafted state lives at `scripts/check-raw-buttons.mjs` with unit tests at
`scripts/check-raw-buttons.test.mjs`. It is **not wired into CI** — activating
it today fails against the existing 44 sites. Phase-3 migration PR 4 flips the
switch per the steps above.

The rule itself is a sibling to `scripts/check-tokens.mjs`: a plain-Node
script, zero new deps, walks `web/packages/app/src/**/*.tsx`, and flags
any JSX `<button` opening tag whose file is not in `ALLOWLIST`. It does not
depend on ESLint's TSX parser because the repo has no ESLint wiring today;
using the same pattern as `check-tokens` keeps the dev dependency surface
flat.
