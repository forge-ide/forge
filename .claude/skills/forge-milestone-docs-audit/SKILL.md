---
name: forge-milestone-docs-audit
description: Use when auditing documentation for a Forge milestone — checks whether every capability shipped under the milestone is documented, whether existing docs have gone stale or inaccurate, whether derived artifacts (like tokens.css vs docs/design/token-reference.md) have drifted from their authoritative source, and whether markdown links still resolve. Produces one GitHub issue per finding plus a consolidated report. Trigger on phrases like "docs audit for Phase N", "check the milestone docs", "are our docs up to date after this phase", "sweep Phase N for missing docs", or whenever milestone-level documentation review is wanted.
---

# forge-milestone-docs-audit

## Overview

Milestone-scoped documentation audit. Scope is the **current state** of every `docs/` subtree and inline-doc surface (rustdoc, tsdoc, README.md) belonging to crates and packages touched by the milestone's merged PRs. Output is **one GitHub issue per finding** plus **one consolidated report issue**. The main context coordinates; scope gathering and per-area audits are delegated.

## Arguments

| Argument | Form | Meaning |
|----------|------|---------|
| Milestone (required) | `"Phase N: Title"` | The GitHub milestone title to audit |

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
> 3. Produce **two paired lists**, both deduped:
>    - **Code areas** (first two path segments of each changed file): `crates/forge_ipc`, `web/packages/app`, `scripts`, etc.
>    - **Doc surfaces implicated** — for each code area, return the matching doc surfaces that *should* describe it. Forge's mapping:
>      - `crates/<crate>` → `docs/architecture/*.md` (by topic), that crate's `README.md`, and its rustdoc
>      - `web/packages/<pkg>` → `docs/frontend/*.md` and that package's `README.md`
>      - Any UI component under `web/packages/app/src/features/*` → `docs/ui-specs/<component>.md`
>      - Design tokens → `docs/design/token-reference.md` (authoritative) vs `web/packages/design/src/tokens.css` (derived)
>      - Any milestone adds a testing story → `docs/testing/phase<N>-uat.md` should cover it
>      - Changes to build, CI, or release shape → `docs/build/*.md`
> 4. Report which doc files exist vs. which the mapping says should exist. Flag **missing mapping rows** as preliminary findings (no final severity yet).
>
> Return: milestone facts, code-areas list, paired doc-surfaces list, preliminary missing-doc list.

### 2. Derive the concern model — invoke `superpowers:brainstorming`

Do not bring a generic checklist. The load-bearing reason is that *what counts as a docs gap* depends on what the milestone actually shipped — a milestone that changed an IPC contract creates completeness concerns for `docs/architecture/ipc-contracts.md` specifically; a milestone that added UI creates concerns for `docs/ui-specs/` and token drift. A generic "do all docs exist" checklist produces noise.

Brainstorm with the user, seeded by Phase 1 findings, to produce a concern model covering:

- **Missing** — what capability shipped without a doc that the mapping implies should exist?
- **Stale** — what docs describe superseded behavior (renamed symbols, removed features, old API shapes)?
- **Inaccurate** — where do docs claim one thing and the code do another? (subagents need to read both sides for this)
- **Broken** — which internal markdown links or symbol references no longer resolve?
- **Drift** — which derived artifacts diverged from their authoritative source? (tokens.css vs token-reference.md is the known example; surface others if the milestone introduced them)
- **Compliance** — does every new UI component have a matching `docs/ui-specs/*.md` entry? Does every new IPC message type appear in `docs/architecture/ipc-contracts.md`?

Output: a bulleted concern model — concern class, one-line rationale for this milestone, and an expected severity ceiling.

**HARD GATE:** Do not begin Step 3 until the concern model has been presented and approved.

### 3. Run automated checks — Bash, parallel

Cheap deterministic signal first. Run whichever apply; skip the rest.

```bash
OUT=/tmp/forge-docs-audit-<milestone-slug>/checks
mkdir -p "$OUT"

# Token drift (authoritative source: docs/design/token-reference.md)
node scripts/check-tokens.mjs              > "$OUT/check-tokens.txt"   2>&1 || true

# Rustdoc build — warnings surface broken intra-doc links and malformed doctests
cargo doc --no-deps --all-features         > "$OUT/cargo-doc.txt"      2>&1 || true

# Markdown link sanity — if `lychee` is installed, otherwise skip
command -v lychee >/dev/null && \
  lychee --offline --no-progress 'docs/**/*.md' 'crates/**/README.md' 'web/packages/**/README.md' \
    > "$OUT/lychee.txt" 2>&1 || true
```

Summarize: count issues per check. Do not dump raw output into the main context.

### 4. Per-area docs audit — invoke `superpowers:dispatching-parallel-agents`

One subagent per (code area + paired doc surface) pair, all dispatched in the same turn. Brief each identically:

> Audit the documentation for this paired code area and doc surface.
>
> **Code area:** `<path>`
> **Doc surfaces (paired):** `<list>`
> **Concern classes to weight:** `<from Step 2>`
>
> Read the code area AND its paired docs. For each *finding*, return an object with these fields exactly:
>
> - `title` — imperative, ≤60 chars (e.g. "Document new `ToolCallEvent::ApprovalRequested` variant")
> - `severity` — critical | high | medium | low
> - `location` — path to the doc that should change, with line range if pointing at an existing section; or the code symbol if arguing a missing doc
> - `concern_class` — missing | stale | inaccurate | broken | drift | compliance
> - `evidence` — *both sides*: what the code says vs what the docs say (or don't say). Quote the relevant lines.
> - `remediation` — concrete edit: which file to touch, which section, what to add/replace
> - `references` — link to ADR / issue / PR if relevant
>
> Rules:
> - Only report findings with **both sides present as evidence** — "this seems undocumented" without a code pointer is not a finding.
> - Do not report `// TODO` comments as docs gaps unless the TODO references a shipped capability that lacks a user-facing doc.
> - If the paired docs are fully in sync, return `[]` with a one-sentence rationale.

Aggregate, deduplicate.

### 5. Triage — invoke `superpowers:brainstorming`

Present the deduped finding list + check summary. Brainstorm:

- Findings to merge (one missing doc that entails several smaller gaps should be one issue)
- False positives to drop
- Severity adjustments (e.g., drift enforced by CI is higher severity than a broken external link)

**HARD GATE:** Do not create any GitHub issues until the triaged list is approved.

### 6. Create finding issues — sequential

Find the next available F-number:

```bash
gh issue list --repo forge-ide/forge --state all --json title \
  --jq '[.[].title | scan("F-([0-9]+)") | .[0] | tonumber] | max + 1'
```

One issue per finding, **sequentially**:

```bash
gh issue create \
  --repo forge-ide/forge \
  --title "[F-NNN] <imperative title>" \
  --milestone "<milestone>" \
  --label "type: bug,docs: <severity>" \
  --body "$(cat <<'EOF'
## Scope

<One paragraph: which code area and doc surface, which concern class, why it matters for this milestone.>

## Finding

- **Location:** `<path>` (or symbol)
- **Severity:** <severity>
- **Concern class:** missing | stale | inaccurate | broken | drift | compliance

### Evidence

**Code says:**
```<language>
<quoted code>
```

**Docs say (or don't say):**
```
<quoted docs, or "no matching section">
```

## Remediation

<Concrete edit — name the file and section, say what to add or replace.>

## References

<ADR / PR / issue links, or "none".>

## Definition of Done

- [ ] <Docs change applied — name the file and section>
- [ ] If the finding was drift-class: rerun the drift check and confirm it passes
- [ ] Tests (and CI doc checks) pass
EOF
)"
```

### 7. Create the consolidated report issue

```
Title:  [F-NNN] Docs audit report: <milestone>
Milestone: <milestone>
Labels: type: bug, docs: audit
```

Body template:

```markdown
## Summary

Milestone: <milestone>
Code areas audited: <N>
Doc surfaces audited: <M>
Findings by severity: critical <a> / high <b> / medium <c> / low <d>
Findings by class: missing <w> / stale <x> / inaccurate <y> / broken <z> / drift <u> / compliance <v>

## Concern model

<Bulleted list from Step 2, verbatim.>

## Scope mapping

| Code area | Paired doc surfaces | Touched by PRs |
|-----------|---------------------|----------------|
| `crates/forge_ipc` | `docs/architecture/ipc-contracts.md`, rustdoc | #41, #52 |
| ... | ... | ... |

## Findings

| ID | Severity | Class | Title | Location | Issue |
|----|----------|-------|-------|----------|-------|
| DOC-01 | high | missing | ... | `docs/architecture/ipc-contracts.md` | #123 |

## Automated checks

- check-tokens: <summary>
- cargo doc: <N> warnings (<summary>)
- lychee (if run): <N> broken links

Raw output: `/tmp/forge-docs-audit-<milestone-slug>/checks/`
```

### 8. Verify — invoke `superpowers:verification-before-completion`

```bash
gh issue list --repo forge-ide/forge \
  --milestone "<milestone>" \
  --search 'label:"docs: audit" OR label:"docs: critical" OR label:"docs: high" OR label:"docs: medium" OR label:"docs: low"' \
  --json number,title,labels
```

Confirm every triaged finding has an open issue with the right label, and the consolidated report exists with table rows that resolve. Do not claim done without this evidence.

## Delegation rules

| Work type | Where it runs |
|-----------|---------------|
| Milestone metadata, PR listing, code-area → doc-surface mapping | `Explore` subagent |
| Concern model derivation | `superpowers:brainstorming` with the user |
| Per-pair docs audit | parallel subagents via `superpowers:dispatching-parallel-agents` |
| Drift / rustdoc / link checks | Bash; results summarized, raw output on disk |
| Finding triage | `superpowers:brainstorming` with the user |
| Issue creation | Main context, strictly sequential |
| Final verification | `superpowers:verification-before-completion` |

## Common mistakes

| Mistake | Correct |
|---------|---------|
| Auditing docs without reading the code | Every finding requires evidence from *both* sides — code and docs — or it is speculation |
| Generic "is this doc complete" pass | Derive concern classes from what the milestone actually shipped |
| Treating `// TODO` as a docs finding | Only if the TODO references a user-facing shipped capability with no doc |
| Skipping `check-tokens.mjs` | Token drift is CI-enforced; it is the cheapest, highest-signal check here |
| Missing UI-spec compliance | Every new feature under `web/packages/app/src/features/*` needs a `docs/ui-specs/*.md` entry |
| Flagging every old doc as stale | Staleness requires contradiction evidence, not age |
| Creating issues in parallel | Sequential only — F-number collisions |
| Dumping link-check raw output into the main context | Summarize counts; keep raw output on disk |
| Claiming done without `gh issue list` evidence | `verification-before-completion` requires it |
