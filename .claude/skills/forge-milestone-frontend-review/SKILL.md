---
name: forge-milestone-frontend-review
description: Use when reviewing frontend changes from a Forge milestone — checks accessibility, design-token adherence (via scripts/check-tokens.mjs), spec compliance against docs/ui-specs/, interaction-state coverage (loading/empty/error/success), design-system consistency against the @forge/design package, and React/TS hygiene. Scope is the current state of every touched web/packages/* area plus any crates defining IPC types that cross into the webview. Produces one GitHub issue per finding plus a consolidated report. Trigger on phrases like "review the frontend for Phase N", "audit the UI changes from this milestone", "check the milestone's frontend", "sweep Phase N for a11y and design issues", or any milestone-level frontend pass.
---

# forge-milestone-frontend-review

## Overview

Milestone-scoped frontend review. Scope is the **current state** of every `web/packages/*` area touched by the milestone's merged PRs, plus neighboring code in the same packages, plus any crate-side IPC types consumed by the webview (the boundary where prop shapes originate). The distinct value vs. PR-level frontend review is seeing how **new UI fits against the design system as a whole** — easy to miss on a single PR but visible when you inspect several components at once.

Output is **one GitHub issue per finding** plus **one consolidated report issue**. Uses `frontend-design:frontend-design` to seed the concern model with the repo's aesthetic direction.

## Arguments

| Argument | Form | Meaning |
|----------|------|---------|
| Milestone (required) | `"Phase N: Title"` | The GitHub milestone title to review |

If the argument is missing, list candidate milestones and ask:

```bash
gh api repos/forge-ide/forge/milestones --jq '.[] | {title, state, open_issues, closed_issues}'
```

## Steps

### 1. Gather scope — invoke `Explore`

Delegate to an `Explore` subagent. Brief:

> For milestone `"<milestone>"`:
>
> 1. Fetch milestone metadata: `gh api repos/forge-ide/forge/milestones --jq '.[] | select(.title == "<milestone>")'`.
> 2. List merged PRs: `gh pr list --repo forge-ide/forge --search 'milestone:"<milestone>" is:merged' --json number,title,files,mergedAt --limit 200`.
> 3. Filter changed files to `web/**` plus any `crates/**` file that defines IPC types consumed by the web (look for types re-exported through `web/packages/ipc/`). Reduce to first-two-segment areas (e.g. `web/packages/app`, `web/packages/design`, `web/packages/ipc`). Dedupe.
> 4. For each frontend area, return: package name, one-line purpose, public entry points, and — for `web/packages/app` specifically — the list of component directories under `src/features/*`.
> 5. For every component directory found under `web/packages/app/src/features/*`, check whether a matching `docs/ui-specs/<component>.md` exists. Return the pairing list (`component path → spec path | MISSING`).
> 6. Read `web/packages/design/src/` — list the primitives exported (`Button`, `Panel`, `Input`, etc.), and report whether any of the milestone's PRs added inline alternatives to these primitives (grep for raw `<button>`, `<input>`, inline styles, etc. in the touched `app` code).
> 7. Check runnable tooling: `pnpm --version`, `pnpm typecheck` feasibility, whether `scripts/check-tokens.mjs` exists.
>
> Return: milestone facts, frontend-areas list, component↔spec pairing, design-system primitives, raw-element sightings, tooling availability.

### 2. Seed aesthetic direction — invoke `frontend-design:frontend-design`

Invoke `frontend-design:frontend-design` not to generate code, but to surface:

- The product's aesthetic direction (this repo has `docs/design/philosophy.md`, `docs/design/component-principles.md`, `docs/design/voice-terminology.md` — read them to ground the review in the established direction, not a generic design sensibility)
- The principles the review should weigh components against
- The interaction-state expectations (loading/empty/error/success) the design system implies

Output: a short **design brief** — the aesthetic direction, the principles, and the state-coverage expectations — in the review's own voice. This grounds the concern model in Step 3.

### 3. Derive the concern model — invoke `superpowers:brainstorming`

The load-bearing reason this is brainstormed: *which frontend concerns matter most* depends on what the milestone shipped. A milestone that added a new UI surface concentrates on spec compliance and novel-component scrutiny; a milestone that refactored shared primitives concentrates on consistency regressions across callers.

Seeded with the Phase 1 scope and the Phase 2 design brief, brainstorm with the user to produce a concern model covering whichever apply:

- **Accessibility** — keyboard nav, focus management, ARIA where needed, sufficient contrast, screen-reader landmarks
- **Design-token adherence** — CSS uses `var(--token-name)` rather than magic numbers; `check-tokens` passes
- **Spec compliance** — every component under `src/features/*` matches its `docs/ui-specs/*.md` entry (or the spec is updated)
- **Interaction-state coverage** — loading, empty, error, success all handled for async-driven UI (the repo's design ethos calls this out)
- **Design-system consistency** — new UI uses `@forge/design` primitives; no parallel re-implementations
- **React/TS hygiene** — `key` prop on lists, effect cleanup, no escaping `any`, no unchecked non-null assertions, controlled inputs wired end-to-end
- **IPC boundary typing** — prop shapes derived from IPC types stay in sync with their crate-side origin (look for ad-hoc types duplicating an IPC shape)
- **Voice and terminology** — component copy consistent with `docs/design/voice-terminology.md`

Output: a bulleted concern model — concern, one-line rationale for this milestone, expected severity ceiling.

**HARD GATE:** Do not begin Step 4 until the concern model is presented and approved.

### 4. Run automated checks — Bash, parallel

```bash
OUT=/tmp/forge-frontend-review-<milestone-slug>/checks
mkdir -p "$OUT"

( cd web && pnpm -r typecheck )                  > "$OUT/typecheck.txt"     2>&1 || true
node scripts/check-tokens.mjs                    > "$OUT/check-tokens.txt"  2>&1 || true
( cd web && pnpm -r build )                      > "$OUT/build.txt"         2>&1 || true
( cd web && pnpm -r test )                       > "$OUT/test.txt"          2>&1 || true
```

Summarize: pass/fail per check, count of issues. Do not dump raw output into the main context.

### 5. Per-area frontend review — invoke `superpowers:dispatching-parallel-agents`

One subagent per frontend area, all dispatched in the same turn. Brief each identically:

> Review this frontend area at milestone scope.
>
> **Area:** `<path>`
> **Purpose:** `<from Step 1>`
> **Component ↔ spec pairings (if any):** `<from Step 1>`
> **Design brief:** `<from Step 2>`
> **Concern classes to weight, in order:** `<from Step 3>`
>
> Read every file in the area. For each component under `src/features/*`, read its paired `docs/ui-specs/*.md` if one exists.
>
> Your distinct job vs. per-PR review: look at components *in relation to each other and to the design system*. Ask: "Does this new UI use the primitives the design system already provides? Does it match the states the design ethos calls out? Does it drift from its spec?"
>
> For each *finding*, return an object with these fields exactly:
>
> - `title` — imperative, ≤60 chars
> - `severity` — critical | high | medium | low
> - `location` — `path:line` (add more sites if cross-file)
> - `concern_class` — from the Step 3 concern model
> - `evidence` — a concrete quote or reference: the offending code AND either the design-system primitive it should have used, the spec it drifts from, or the interaction state it missed
> - `remediation` — concrete fix: use which primitive, update which spec, add which state
> - `estimated_effort` — small | medium | large
>
> Rules:
> - Every finding must cite a concrete comparison target — primitive, spec, token, or state expectation. "Looks wrong" is not a finding.
> - Do not report typecheck errors covered by the scanners.
> - If the area matches the design brief and concern model cleanly, return `[]` with a one-sentence rationale.

Aggregate, deduplicate.

### 6. Triage — invoke `superpowers:brainstorming`

Present the deduped finding list + scanner summary. Brainstorm:

- Merges (one pattern found across several components → one issue)
- Severity adjustments — the user knows which flows are user-facing critical vs internal
- Spec compliance: when the component is right and the spec is stale, the fix is to update the spec — not the code (surface this explicitly so the DoD points at the right file)
- Issue-creation ordering

**HARD GATE:** Do not create any GitHub issues until the triaged list is approved.

### 7. Create finding issues — sequential

Find the next F-number (same pattern as `forge-create-task`), then:

```bash
gh issue create \
  --repo forge-ide/forge \
  --title "[F-NNN] <imperative title>" \
  --milestone "<milestone>" \
  --label "type: bug,frontend: <severity>" \
  --body "$(cat <<'EOF'
## Scope

<One paragraph: which area/component, which concern class, how it reads against the design brief.>

## Finding

- **Location:** `<path:line>` (list all sites for cross-file patterns)
- **Severity:** <severity>
- **Concern class:** <class from concern model>
- **Estimated effort:** small | medium | large

### Evidence

**Current code:**
```tsx
<quoted code>
```

**Expected per design system / spec / state model:**
<primitive to use, or spec excerpt, or state expectation>

## Remediation

<Concrete fix — use which primitive, update which spec, add which state.>

## Definition of Done

- [ ] <Fix applied — name the symbol/file/spec>
- [ ] <Test added (interaction, a11y, or visual as appropriate)>
- [ ] `pnpm -r typecheck` clean
- [ ] `node scripts/check-tokens.mjs` passes (if tokens involved)
- [ ] Tests pass in CI
EOF
)"
```

### 8. Create the consolidated report issue

```
Title:  [F-NNN] Frontend review report: <milestone>
Milestone: <milestone>
Labels: type: bug, frontend: audit
```

Body template:

```markdown
## Summary

Milestone: <milestone>
Frontend areas reviewed: <N>
Components with specs / without specs: <X> / <Y>
Findings by severity: critical <a> / high <b> / medium <c> / low <d>
Findings by class: <counts per concern class>

## Design brief

<From Step 2, verbatim.>

## Concern model

<From Step 3, verbatim.>

## Scope

| Area | Purpose | Components | Touched by PRs |
|------|---------|-----------|----------------|
| `web/packages/app` | main app | 5 features | #41, #52 |
| ... | ... | ... | ... |

## Component ↔ spec pairings

| Component path | Spec | Status |
|----------------|------|--------|
| `src/features/session` | `docs/ui-specs/session-roster.md` | paired |
| `src/features/new-thing` | — | MISSING SPEC |

## Findings

| ID | Severity | Class | Title | Area | Effort | Issue |
|----|----------|-------|-------|------|--------|-------|
| FE-01 | high | design-system | ... | `web/packages/app/src/features/session` | small | #123 |

## Automated checks

- pnpm typecheck: <status>
- check-tokens: <status>
- pnpm build: <status>
- pnpm test: <status>

Raw output: `/tmp/forge-frontend-review-<milestone-slug>/checks/`
```

### 9. Verify — invoke `superpowers:verification-before-completion`

```bash
gh issue list --repo forge-ide/forge \
  --milestone "<milestone>" \
  --search 'label:"frontend: audit" OR label:"frontend: critical" OR label:"frontend: high" OR label:"frontend: medium" OR label:"frontend: low"' \
  --json number,title,labels
```

Confirm every triaged finding has an open issue with the right label, and the consolidated report exists. Do not claim done without this evidence.

## Delegation rules

| Work type | Where it runs |
|-----------|---------------|
| Milestone metadata, PR listing, scope + component/spec pairing | `Explore` subagent |
| Design brief / aesthetic direction | `frontend-design:frontend-design` |
| Concern model derivation | `superpowers:brainstorming` with the user |
| Per-area frontend review | parallel subagents via `superpowers:dispatching-parallel-agents` |
| Scanner invocations | Bash; results summarized, raw output on disk |
| Finding triage | `superpowers:brainstorming` with the user |
| Issue creation | Main context, strictly sequential |
| Final verification | `superpowers:verification-before-completion` |

## Common mistakes

| Mistake | Correct |
|---------|---------|
| Reviewing components in isolation | The cross-milestone value is seeing *consistency* across components and against the design system |
| Skipping `frontend-design:frontend-design` | Without the design brief the concern model collapses into generic "is this nice-looking" — not actionable |
| Generic a11y checklist | Derive the a11y concerns from what the milestone actually added (forms? dialogs? menus?) |
| Inline re-implementing a design-system primitive | Flag it — the `@forge/design` package exists precisely to prevent this |
| Flagging spec drift as a code bug | If the component is right and the spec is stale, the DoD should name the spec file to update |
| Ignoring IPC-type drift at the boundary | Prop shapes duplicated at the webview boundary rot fast; flag them |
| Treating voice/terminology as nit-tier | It's a listed concern because `docs/design/voice-terminology.md` is authoritative |
| Reporting typecheck errors as findings | Scanners already ran; findings should be things scanners can't see |
| Creating issues in parallel | Sequential only — F-number collisions |
| Claiming done without `gh issue list` evidence | `verification-before-completion` requires it |
