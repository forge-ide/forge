---
name: forge-milestone-release-readiness
description: Use when verifying a Forge milestone is ready to ship — checks whether the changelog covers every merged PR, whether the version is bumped appropriately for the scope of change (semver), whether breaking changes have migration notes, whether the UAT script under docs/testing/phase-N-uat covers the milestone's deliverables, whether any open issues tagged critical/high are still blocking, and whether CI on main is green. This is the final-gate skill — run it after the other milestone audits so it can surface their unresolved findings as blockers. Trigger on phrases like "is Phase N ready to release", "release-readiness check for the milestone", "pre-release audit", "blockers before cutting the release", or any pre-release go/no-go gate.
---

# forge-milestone-release-readiness

## Overview

The **final-gate** milestone audit. Scope is not the code — it is the **release artifacts**: changelog, version, release notes, UAT coverage, outstanding blockers, and CI state. This skill depends on the other milestone audits having been run; it surfaces any unresolved critical/high findings from them as release blockers.

Unlike the code-auditing siblings, this skill is **mostly mechanical** — the checks are fixed (changelog, version, notes, UAT, blockers, CI). It still uses brainstorming, but for a narrower purpose: deciding whether what's missing is actually a blocker *for this milestone specifically* (some milestones don't need migration notes; some don't bump the version).

Output: **one GitHub issue per missing artifact / unresolved blocker** (labeled `type: bug, release: <severity>`), plus **one consolidated go/no-go report** (labeled `type: bug, release: audit`).

## Arguments

| Argument | Form | Meaning |
|----------|------|---------|
| Milestone (required) | `"Phase N: Title"` | The GitHub milestone title to assess |

If the argument is missing, list candidate milestones and ask:

```bash
gh api repos/forge-ide/forge/milestones --jq '.[] | {title, state, open_issues, closed_issues}'
```

## Steps

### 1. Gather release artifacts — invoke `Explore`

Delegate to an `Explore` subagent. Brief:

> For milestone `"<milestone>"`, collect release-readiness evidence:
>
> 1. **Milestone stats**: `gh api repos/forge-ide/forge/milestones --jq '.[] | select(.title == "<milestone>")'`. Return open vs. closed counts; a milestone with open issues is generally not ready.
> 2. **Merged PRs**: `gh pr list --repo forge-ide/forge --search 'milestone:"<milestone>" is:merged' --json number,title,body,mergedAt,labels,mergeCommit --limit 200`. Collect titles, bodies, and labels (so we can see which are feat vs chore vs bug).
> 3. **Open issues under the milestone**: `gh issue list --repo forge-ide/forge --milestone "<milestone>" --state open --json number,title,labels`. These are unfinished work.
> 4. **Outstanding audit blockers** — issues created by the sibling milestone audits that are still open and labeled critical or high: `gh issue list --repo forge-ide/forge --milestone "<milestone>" --state open --search 'label:"security: critical" OR label:"security: high" OR label:"docs: critical" OR label:"quality: critical" OR label:"quality: high" OR label:"frontend: critical" OR label:"perf: critical" OR label:"perf: high" OR label:"compat: critical" OR label:"compat: high"' --json number,title,labels`.
> 5. **Changelog**: look for `CHANGELOG.md` (repo root or `docs/`). If it exists, read the section that should cover this milestone.
> 6. **Versions**: read `Cargo.toml` workspace `version` and every `package.json` version under `web/`. Note any baseline (last release tag) for comparison: `git describe --tags --abbrev=0` if tags exist.
> 7. **UAT doc**: `docs/testing/phase<N>-uat.md` and the matching `phase<N>-uat.sh` — report existence, last-modified date, and whether it was touched during this milestone's time window.
> 8. **CI state on main**: `gh run list --repo forge-ide/forge --branch main --limit 5 --json status,conclusion,workflowName,createdAt`.
> 9. **Release notes draft**: any file under `docs/` or the repo root matching `release-notes*`, `RELEASE_NOTES*`, or similar.
>
> Return: a structured summary grouped as {milestone stats, PR list with labels, open-issue count, audit-blocker list, changelog excerpt (or MISSING), version(s), UAT status, CI status, release-notes status}.

### 2. Derive the go/no-go criteria — invoke `superpowers:brainstorming`

Release readiness is not universal — what *must* be done before cutting this milestone's release depends on what the milestone actually did.

The load-bearing reason this is brainstormed and not a fixed checklist: a milestone that only refactored internals doesn't need migration notes; a Phase-0 foundation milestone pre-1.0 probably doesn't bump the version the way a post-1.0 feature would; a milestone whose UAT was manual doesn't need a script.

Use brainstorming with the user, seeded by Phase 1 evidence, to settle:

- **Changelog expectation**: is it expected for this milestone? What entry shape? (if none exists, is that OK?)
- **Version-bump expectation**: major / minor / patch / none (pre-1.0 rules differ)? Should the workspace version change? Which `package.json` versions?
- **Breaking changes**: did the milestone introduce any? If yes, migration notes are a hard requirement
- **UAT expectation**: does a `phase<N>-uat.md` exist, is it current, does it cover the merged PRs?
- **Open-issue tolerance**: must *every* milestone issue be closed, or are some deferrable? (the user knows which)
- **Audit-blocker policy**: which severity levels block release? (default: critical + high)

Output: a checklist of concrete release-readiness criteria for *this* milestone, each phrased as a pass/fail test.

**HARD GATE:** Do not begin Step 3 until the criteria list is approved. Shipping on vibes is how bugs escape.

### 3. Run mechanical checks — invoke `superpowers:dispatching-parallel-agents`

These artifacts are independent; dispatch a subagent per artifact class in the same turn. Each subagent checks one criterion and returns pass/fail with evidence.

**Subagent A — Changelog coverage:**
> Given the merged-PR list (titles + numbers) and the changelog excerpt from Phase 1, verify every PR has a corresponding changelog entry. Return a table of `PR # | title | entry present | notes`. If no changelog exists and the criteria require one, flag as failing with severity per the criteria.

**Subagent B — Version correctness:**
> Given the current `Cargo.toml` / `package.json` versions and the version-bump expectation from the criteria, verify:
> - The workspace version (if expected to change) moved in the right direction
> - Per-package versions moved consistently
> - No accidental downgrades
> Return pass/fail with evidence. If the criteria say no bump is needed, confirm versions are unchanged.

**Subagent C — UAT coverage:**
> Given `docs/testing/phase<N>-uat.md` (if present) and the merged-PR list, verify every PR's deliverable is covered by at least one UAT step. Flag undiscoverable/untested deliverables. If the UAT file is missing or stale relative to the milestone's time window, say so.

**Subagent D — Migration / breaking-change notes:**
> Given the PR list and their bodies, identify PRs that introduce breaking changes (look for label `breaking-change` if used, or PR body sections labeled "Breaking" / "Migration"). For each, verify a migration note exists — in the changelog, in release-notes draft, or in an ADR under `docs/architecture/`. Flag any breaking PR without a migration note.

**Subagent E — Blocker sweep:**
> Given the audit-blocker list from Phase 1 (open issues labeled critical/high under the milestone), for each issue return: number, title, severity label, and one-line status summary from its body. These are release blockers; the report will list them.

**Subagent F — CI sanity:**
> Given the CI run list from Phase 1, verify the latest main CI run is `success`. If the latest is `failure` / `in_progress` / `cancelled`, that is a blocker. Return the latest run's state and link.

Aggregate all six returns.

### 4. Consolidate and decide — invoke `superpowers:brainstorming`

Present all check results side-by-side to the user. Each becomes either:
- **Pass** — recorded in the report, no issue needed
- **Blocker** — becomes a `type: bug, release: <severity>` issue
- **Waived** — the user says "ship anyway" with a recorded rationale in the report

The go/no-go call is the user's, not the skill's. But the skill must surface every failing criterion and require an explicit waive decision — silent passing is what lets bad releases out.

**HARD GATE:** Do not create any GitHub issues until every failing criterion has been classified (blocker vs. waived) and the decision is recorded.

### 5. Create blocker issues — sequential

One issue per classified blocker. Find the next F-number, then:

```bash
gh issue create \
  --repo forge-ide/forge \
  --title "[F-NNN] <imperative title>" \
  --milestone "<milestone>" \
  --label "type: bug,release: <severity>" \
  --body "$(cat <<'EOF'
## Scope

<One paragraph: which release-readiness criterion failed and why it blocks ship.>

## Criterion

- **Criterion:** <from Step 2>
- **Severity:** <severity>
- **Check subagent:** <A|B|C|D|E|F>

### Evidence

<Concrete failing evidence — missing changelog entry for PR #N, untouched UAT doc, critical security finding #M still open, etc.>

## Remediation

<Concrete fix — write which entry, bump which version, complete which issue, fix CI, etc.>

## Definition of Done

- [ ] <Concrete action>
- [ ] Release-readiness skill re-run and this criterion passes
EOF
)"
```

### 6. Create the consolidated go/no-go report

```
Title:  [F-NNN] Release readiness report: <milestone>
Milestone: <milestone>
Labels: type: bug, release: audit
```

Body template:

```markdown
## Decision

**<GO | NO-GO | GO WITH WAIVERS>**

<One-paragraph rationale.>

## Criteria (from Step 2)

<Bulleted list, verbatim.>

## Check results

| Check | Result | Blocker? | Issue |
|-------|--------|----------|-------|
| Changelog coverage | pass | — | — |
| Version correctness | fail | blocker | #123 |
| UAT coverage | fail | waived (see below) | — |
| Migration notes | pass | — | — |
| Audit blockers | fail (3 open) | blockers | #124, #125, #126 |
| CI green on main | pass | — | — |

## Waivers

- **UAT coverage**: <rationale for waiving — who made the call, why it is acceptable for this milestone>

## Open blockers

| Issue | Severity | Title |
|-------|----------|-------|
| #123 | high | ... |
| #124 | critical | (from security audit) ... |

## Versions at release

- Workspace (Cargo.toml): <version>
- web/packages/app: <version>
- ... (list relevant packages)

## CI at release candidate

- Latest main run: <status> — <link>
```

### 7. Verify — invoke `superpowers:verification-before-completion`

```bash
gh issue list --repo forge-ide/forge \
  --milestone "<milestone>" \
  --search 'label:"release: audit" OR label:"release: critical" OR label:"release: high" OR label:"release: medium" OR label:"release: low"' \
  --json number,title,labels
```

Confirm every classified blocker has an issue, the go/no-go report exists, and every waived criterion is named in the waivers section. Do not claim done without this evidence.

## Delegation rules

| Work type | Where it runs |
|-----------|---------------|
| Artifact gathering (PRs, issues, changelog, versions, UAT, CI) | `Explore` subagent |
| Go/no-go criteria derivation | `superpowers:brainstorming` with the user |
| Per-criterion mechanical checks | six parallel subagents via `superpowers:dispatching-parallel-agents` |
| Classifying failures (blocker vs. waived) | `superpowers:brainstorming` with the user |
| Blocker-issue creation | Main context, strictly sequential |
| Final verification | `superpowers:verification-before-completion` |

## Common mistakes

| Mistake | Correct |
|---------|---------|
| Running this skill before the sibling audits | Release-readiness surfaces their findings as blockers — run it *after* security/docs/quality/frontend/perf/compat |
| Silent passing on a failing criterion | Every failing criterion must be classified blocker or waived-with-rationale — never silently waived |
| Hard-coding "every PR needs a changelog entry" | Some milestones don't; derive the criteria per-milestone in Step 2 |
| Treating pre-1.0 version bumps like post-1.0 | The rules differ; the criteria step settles which regime applies |
| Skipping the waiver rationale | Waivers without recorded reasons become future mysteries |
| Declaring GO with an unreviewed waiver | The user makes the call; the skill surfaces the decision |
| Creating issues in parallel | Sequential only — F-number collisions |
| Forgetting to record the baseline state in the report | The report is the release's provenance — versions and CI run must be captured |
| Claiming done without `gh issue list` evidence | `verification-before-completion` requires it |
