---
name: forge-milestone-quality-review
description: Use when reviewing code quality across an entire Forge milestone — catches cross-PR patterns that per-PR review misses, like parallel APIs for the same concept accumulated across several PRs, duplicated logic, test-coverage gaps in critical paths, architectural drift, inconsistent error handling, and missing observability. Runs cargo clippy, cargo fmt --check, and pnpm typecheck as cheap scanners. Produces one GitHub issue per finding plus a consolidated report. Trigger on phrases like "quality review for Phase N", "audit the milestone for code quality", "check the milestone for tech debt", "sweep Phase N for inconsistencies", or any milestone-level quality pass.
---

# forge-milestone-quality-review

## Overview

Milestone-scoped code-quality review. Scope is the **current state** of every crate and top-level area touched by the milestone's merged PRs, plus adjacent code in the same crates. The distinct value is **cross-PR pattern detection** — individual PR review looks at one diff at a time and cannot see parallel APIs, duplicated logic, or architectural drift that only becomes visible across several merges together. Output is **one GitHub issue per finding** plus **one consolidated report issue**.

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
> 3. Reduce the union of changed file paths to their first two path segments. Dedupe. This is the **touched-areas list**.
> 4. For each area, return: package name, one-line purpose, public entry points (`lib.rs`, `src/index.ts`, etc.), and which *other* areas in the list it depends on or is depended on by. Cross-area dependencies are where architectural drift tends to show up.
> 5. Sample 3–5 PRs from the milestone and note what each added or changed. You are not doing the review yet — just building a *feel* for what themes run across the milestone. Return a one-line summary of the recurring themes (e.g. "many PRs touched IPC event dispatching", "repeated work on session state management").
> 6. Check which scanners are runnable: `cargo clippy --version`, `cargo fmt --version`, `pnpm --version`. Report available scanners and their scope.
>
> Return: milestone facts, touched-areas list with cross-area dependency hints, recurring themes, scanner availability.

### 2. Derive the concern model — invoke `superpowers:brainstorming`

The load-bearing reason this step is not a fixed checklist: the point of a milestone-level review is to catch *emergent* patterns — and which patterns are relevant depends on what the milestone was about. A milestone that built a new subsystem has different quality concerns (API design, abstraction boundaries) than a milestone of incremental fixes (consistency, regressions).

Brainstorm with the user, seeded by Phase 1 findings (especially the recurring themes), to produce a concern model covering whichever of these apply:

- **API consistency across PRs** — did multiple PRs introduce parallel ways to do the same thing? (e.g. two shapes of IPC dispatch, two session-state stores, two error types for the same condition)
- **Duplicated logic** — code that could be shared but was rewritten in parallel files
- **Abstraction boundaries** — leaks across crate or layer boundaries (IPC types in UI code, DB types in domain code)
- **Error handling consistency** — same failure handled differently in different code paths without justification
- **Test coverage gaps** — critical paths added without tests, or tests that only cover happy paths
- **Observability** — new flows that should log but don't
- **Dead code** — symbols shipped, then orphaned by a later PR in the same milestone
- **Rust-specific** — `unwrap`/`expect` on untrusted input, `Clone` on large structs in hot paths, unnecessary `Arc`/`Mutex`
- **TS-specific** — `any` escapes, unchecked non-null assertions, prop-drilling that a context would clarify
- **Architectural drift** — new code that violates patterns established earlier in the project

Output: a bulleted concern model — concern, one-line rationale for this milestone, expected severity ceiling.

**HARD GATE:** Do not begin Step 3 until the concern model is presented and approved.

### 3. Run automated scanners — Bash, parallel

Cheap deterministic signal first. Run whichever Step 1 reported as available.

```bash
OUT=/tmp/forge-quality-review-<milestone-slug>/scanners
mkdir -p "$OUT"

cargo fmt --all -- --check                                       > "$OUT/fmt.txt"       2>&1 || true
cargo clippy --all-targets --all-features -- -D warnings         > "$OUT/clippy.txt"    2>&1 || true
( cd web && pnpm -r typecheck )                                  > "$OUT/typecheck.txt" 2>&1 || true
cargo test --no-run --all-targets                                > "$OUT/build.txt"     2>&1 || true
```

Summarize: count findings per scanner. Do not dump raw output into the main context.

### 4. Per-area quality review — invoke `superpowers:dispatching-parallel-agents`

One subagent per touched area, all dispatched in the same turn. Brief each identically:

> Quality-review this code area at milestone scope.
>
> **Area:** `<path>`
> **Purpose:** `<from Step 1>`
> **Concern classes to weight, in order:** `<from Step 2>`
> **Recurring milestone themes:** `<from Step 1>`
>
> Your distinct job vs. per-PR review: find patterns that only emerge when you see *multiple PRs' worth of change together*. Compare files against each other. Ask: "Is there more than one way to do <thing> in this milestone's changes? Is that justified?"
>
> Read every file in the area. Read the tests. For each *finding*, return an object with these fields exactly:
>
> - `title` — imperative, ≤60 chars
> - `severity` — critical | high | medium | low
> - `location` — `path:line` (or multiple if a cross-file pattern — list all)
> - `concern_class` — from the Step 2 concern model
> - `description` — *what is wrong in this codebase specifically*, with at least one concrete code reference. If it is a cross-PR pattern, name the pattern and show both sides.
> - `remediation` — concrete fix: unify to which shape? move which code where? add which test?
> - `estimated_effort` — small | medium | large (guide to help the user prioritize)
>
> Rules:
> - Only report findings you can defend against a skeptical reviewer. Do **not** pad with style or lint nits — `cargo clippy` and `cargo fmt` already ran.
> - If a "finding" is fully covered by one of those scanners' output, skip it here.
> - If the area is clean under the concern model, return `[]` with a one-sentence rationale.

Aggregate, deduplicate — cross-file patterns may surface from multiple subagents reporting the same underlying issue.

### 5. Triage — invoke `superpowers:brainstorming`

Present the deduped finding list + scanner summary. Brainstorm:

- Merges: several findings may be one pattern with multiple instances
- Severity adjustments — context the subagents lacked
- Effort vs. value — what should block the release cut vs. what goes on the debt backlog
- Issue-creation ordering

**HARD GATE:** Do not create any GitHub issues until the triaged list is approved.

### 6. Create finding issues — sequential

Find the next F-number (same pattern as `forge-create-task`), then:

```bash
gh issue create \
  --repo forge-ide/forge \
  --title "[F-NNN] <imperative title>" \
  --milestone "<milestone>" \
  --label "type: bug,quality: <severity>" \
  --body "$(cat <<'EOF'
## Scope

<One paragraph: which area(s), which concern class, why this matters at milestone scope (cross-PR pattern, not a single-PR nit).>

## Finding

- **Location:** `<path:line>` (list all sites for cross-file patterns)
- **Severity:** <severity>
- **Concern class:** <class from concern model>
- **Estimated effort:** small | medium | large

<Description — what is wrong, with code references. If cross-PR pattern, name the pattern and show both shapes.>

## Remediation

<Concrete fix — unify to which shape, move which code where, add which test.>

## Definition of Done

- [ ] <Fix applied — name the symbol/file>
- [ ] <Test added or updated that would have caught this>
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` clean
- [ ] `pnpm -r typecheck` clean (if TS touched)
- [ ] Tests pass in CI
EOF
)"
```

### 7. Create the consolidated report issue

```
Title:  [F-NNN] Quality review report: <milestone>
Milestone: <milestone>
Labels: type: bug, quality: audit
```

Body template:

```markdown
## Summary

Milestone: <milestone>
Areas reviewed: <N>
Findings by severity: critical <a> / high <b> / medium <c> / low <d>
Findings by class: <counts per concern class>

## Concern model

<Bulleted list from Step 2, verbatim.>

## Recurring milestone themes

<From Step 1, verbatim.>

## Scope

| Area | Purpose | Touched by PRs |
|------|---------|----------------|
| `crates/forge_ipc` | IPC framing | #41, #52, #78 |
| ... | ... | ... |

## Findings

| ID | Severity | Class | Title | Area | Effort | Issue |
|----|----------|-------|-------|------|--------|-------|
| Q-01 | high | api-consistency | ... | `crates/forge_ipc` | medium | #123 |

## Automated scanners

- cargo fmt: <N> deltas
- cargo clippy: <N> lints (<top offenders>)
- pnpm typecheck: <N> errors
- cargo test build: <status>

Raw output: `/tmp/forge-quality-review-<milestone-slug>/scanners/`
```

### 8. Verify — invoke `superpowers:verification-before-completion`

```bash
gh issue list --repo forge-ide/forge \
  --milestone "<milestone>" \
  --search 'label:"quality: audit" OR label:"quality: critical" OR label:"quality: high" OR label:"quality: medium" OR label:"quality: low"' \
  --json number,title,labels
```

Confirm every triaged finding has an open issue with the right label, and the consolidated report exists. Do not claim done without this evidence.

## Delegation rules

| Work type | Where it runs |
|-----------|---------------|
| Milestone metadata, PR listing, theme sampling, scanner detection | `Explore` subagent |
| Concern model derivation | `superpowers:brainstorming` with the user |
| Per-area review | parallel subagents via `superpowers:dispatching-parallel-agents` |
| Scanner invocations | Bash; results summarized, raw output on disk |
| Finding triage | `superpowers:brainstorming` with the user |
| Issue creation | Main context, strictly sequential |
| Final verification | `superpowers:verification-before-completion` |

## Common mistakes

| Mistake | Correct |
|---------|---------|
| Duplicating per-PR review | The distinct value is **cross-PR patterns** — look for things visible only when several PRs are read together |
| Reporting clippy/fmt findings | Scanners already ran; the finding list should be things scanners can't see |
| Style nits dressed as quality findings | Every finding must name a concrete cross-file pattern or a gap the scanners missed |
| Vague remediation ("refactor this") | Name the symbol, the target shape, or the unification point |
| Treating large-effort findings the same as small ones | Tag effort; triage may defer large fixes to a debt backlog issue |
| Skipping cross-area dependency review | Cross-area leaks (e.g. IPC types in UI code) are a core milestone-scope signal |
| Creating issues in parallel | Sequential only — F-number collisions |
| Claiming done without `gh issue list` evidence | `verification-before-completion` requires it |
| Skipping the consolidated report | Per-finding issues lose the milestone-level picture |
