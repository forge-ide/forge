---
name: forge-complete-milestone
description: Use when running the end-of-milestone workflow for a Forge milestone — orchestrates the code audits (security/docs/quality/frontend), UAT authorship, regression gates (performance/backcompat), and the release-readiness go/no-go decision in the right order, with user checkpoints between phases so you can pause on any critical finding and resume later. This is the milestone-level analogue of forge-complete-task. Trigger on phrases like "close out Phase N", "run the end-of-milestone workflow", "wrap up the milestone", "ship Phase N", or whenever the user wants to kick off the milestone-completion gauntlet rather than invoke individual audit skills one-by-one.
---

# forge-complete-milestone

## Overview

End-to-end milestone-completion workflow. Four phases in a load-bearing order: **audits → UAT → regression gates → release-readiness**. The main context is a coordination layer — delegate every discrete unit of work to the child skills. Do not re-implement their logic inline.

This skill is the milestone-level analogue of `forge-complete-task`. Like that skill, it runs in phases with explicit gates between them, and the user can pause after any phase.

## Arguments

| Argument | Form | Meaning |
|----------|------|---------|
| Milestone (required) | `"Phase N: Title"` | The GitHub milestone to close out |
| Skip (optional) | `--skip <skill-name>[,<skill-name>...]` | Named skills to skip (e.g. `--skip perf,backcompat`) |
| Only (optional) | `--only <skill-name>[,<skill-name>...]` | Run only named skills, in the listed order |

Resolve skill names to the short form of each child skill: `security`, `docs`, `quality`, `frontend`, `uat`, `perf`, `backcompat`, `release`.

## Resumability

Every child skill produces a **consolidated report issue** with a distinctive audit label. Before invoking any child skill, check whether its report exists under this milestone — if it does, the audit was already run, so skip unless the user explicitly opts in to re-run.

```bash
gh issue list --repo forge-ide/forge --milestone "<milestone>" \
  --search 'label:"<domain>: audit"' --json number,title --limit 1
```

Labels to check: `security: audit`, `docs: audit`, `quality: audit`, `frontend: audit`, `perf: audit`, `compat: audit`, `release: audit`. UAT is detected differently — see Phase B.

Resuming an interrupted run is automatic: re-invoke with the same milestone argument and previously-completed phases will be skipped.

## Auto-skip conditions

Detect these with `Explore` at the start of Phase 0; they save the user from manually passing `--skip`:

- **frontend review** — skip if no file under `web/` was touched by any PR merged under this milestone
- **backcompat audit** — skip if no interface surface file changed (the child skill's Phase 1 will detect this; we can pre-detect too by diffing the milestone-start baseline against current on the interface files listed in `docs/architecture/ipc-contracts.md`)
- **UAT authorship** — skip if `docs/testing/phase<N>-uat.md` already exists and was last touched during this milestone's time window, OR a PR titled `docs(testing): Phase <N> UAT ...` is open/merged

## Phase 0: Setup — invoke `Explore`

Delegate once to build the workflow plan:

> For milestone `"<milestone>"`:
>
> 1. Fetch milestone metadata and state.
> 2. List open vs. closed issues under the milestone. Flag if open-issue count is non-trivial — the workflow can still run, but incomplete milestones may produce noisy UAT plans and premature release-readiness decisions.
> 3. List merged PRs. Compute which top-level areas were touched.
> 4. Derive **auto-skip decisions**: any `web/` touched? (drives frontend-review). Any interface-contract file changed? (drives backcompat). Does `docs/testing/phase<N>-uat.md` exist with a recent mtime, or is a UAT PR open? (drives UAT).
> 5. For each of the eight child skills, check whether its report already exists (see Resumability). Return a **per-skill status**: `ready | done | auto-skip | user-skip`.
> 6. Verify the working tree is clean. Return `git status --short` output.
>
> Return: milestone facts, per-skill status table, working-tree state.

Present the plan to the user: "Here are the eight child skills, N already done, M auto-skipped, K ready to run. Proceed?"

**Gate:** Wait for user acknowledgement before Phase A begins.

## Phase A: Code audits

These four can run in any order — they are independent of each other. Run them sequentially because each one involves the user in its own brainstorming steps; parallel invocation would compete for user attention.

Default order (security first so its critical findings surface early; docs last as least likely to block):

1. `forge-milestone-security-audit "<milestone>"`
2. `forge-milestone-quality-review "<milestone>"`
3. `forge-milestone-frontend-review "<milestone>"` (auto-skipped if no `web/` changed)
4. `forge-milestone-docs-audit "<milestone>"`

**Between each audit, present a checkpoint:**

> Audit complete: <skill>
> - New issues: <count by severity>
> - Highest severity opened: <critical | high | medium | low | none>
> - Critical/high findings: <list titles inline>
>
> Next: <next skill>. Continue, pause, or skip this next audit?

Pause semantics: "pause" stops the workflow here. The user can resume later by re-invoking this skill; completed audits are auto-skipped.

**Hard gate on critical findings:** if any audit opens a `critical` issue, do *not* auto-advance — always ask the user explicitly. Critical findings often change downstream decisions (e.g. a critical security issue may make release-readiness a guaranteed no-go, at which point running the rest is wasted effort).

## Phase B: UAT authorship

Invoke `forge-milestone-uat-author "<milestone>"` unless auto-skipped.

Rationale for ordering after Phase A: the UAT plan should be authored *after* the milestone's defects have been triaged — that way the plan's "Known gap" section can reference already-filed audit issues, and the plan doesn't need rewriting when audits surface blocked deliverables.

**Gate after UAT:** verify the UAT PR was opened (Phase A doesn't need this since audits produce issues in-place). Confirm with:

```bash
gh pr list --repo forge-ide/forge --head "docs/phase<N>-uat" --json number,title,state
```

If the PR does not exist, stop — something went wrong in the UAT skill; investigate before proceeding.

## Phase C: Regression gates

These two need a **clean working tree** because they `git checkout` to the milestone-start baseline for measurement.

Precondition check:

```bash
git status --short
```

If the tree is not clean, present the dirty state to the user and require them to commit or stash before proceeding. Do **not** run `git stash` or `git reset` automatically — unexpected destructive operations on user state violate the coordinator-only contract.

Then:

1. `forge-milestone-performance-audit "<milestone>"`
2. `forge-milestone-backcompat-audit "<milestone>"`

**Serial, not parallel** — both skills check out the baseline SHA. Running them simultaneously would produce a checkout race.

**Between each, same checkpoint format as Phase A.**

Post-Phase-C sanity check: confirm the working tree is back on the original HEAD. Both child skills are supposed to return to HEAD after measurement, but a failed skill can leave the tree detached. Verify:

```bash
git rev-parse HEAD
git status --short
```

If the tree is not back on the expected HEAD, surface the discrepancy — do not continue into Phase D on a detached tree.

## Phase D: Release-readiness

Invoke `forge-milestone-release-readiness "<milestone>"` last.

This is the final gate. It will surface every unresolved critical/high finding from Phases A and C as a blocker, and verify the UAT plan from Phase B covers every deliverable. Running it last is the whole point — its inputs are the outputs of everything before it.

The release-readiness skill produces a go/no-go decision in its consolidated report. Present that decision to the user as this workflow's final output.

## Post-workflow summary

After Phase D, present a one-paragraph summary:

- Milestone: `<name>`
- Phases run: `<list>` (with skipped phases noted)
- Total new issues opened: `<count>` (link to `gh issue list --milestone "<m>" --search 'label:"type: bug" OR label:"type: security"' state:open`)
- UAT PR: `<link or "existed already">`
- Release decision: **GO | NO-GO | GO WITH WAIVERS**
- If NO-GO: the blocking issues

## Delegation rules

| Phase | Delegated to |
|-------|--------------|
| 0 | `Explore` subagent |
| A.1 | `forge-milestone-security-audit` |
| A.2 | `forge-milestone-quality-review` |
| A.3 | `forge-milestone-frontend-review` |
| A.4 | `forge-milestone-docs-audit` |
| B | `forge-milestone-uat-author` |
| C.1 | `forge-milestone-performance-audit` |
| C.2 | `forge-milestone-backcompat-audit` |
| D | `forge-milestone-release-readiness` |
| Checkpoints / pauses | Main context (user-facing coordination only) |
| Final summary | Main context |

## Common mistakes

| Mistake | Correct |
|---------|---------|
| Re-implementing any child-skill step inline | The main context here is *only* a coordinator — every unit of work is delegated |
| Running phases out of order | The order is load-bearing: Phase D consumes Phase A/B/C outputs; reordering breaks the go/no-go gate's dependencies |
| Running Phase C on a dirty tree | Perf and backcompat do `git checkout` to baseline; a dirty tree risks losing uncommitted work — require clean tree explicitly |
| Auto-stashing or auto-resetting user state | Never run destructive git operations to "clean up" — surface the dirty state and let the user decide |
| Running perf and backcompat in parallel | Both check out the baseline SHA; simultaneous runs race each other — serial only |
| Forcing the user past a critical finding | On any `critical`-severity issue from an audit, require explicit user acknowledgement before continuing — don't auto-advance |
| Ignoring auto-skip conditions | A frontend review for a backend-only milestone produces noisy empty findings; detect and skip in Phase 0 |
| Re-running completed audits on resume | Check for each skill's report-issue label before invoking; idempotence is the resumption mechanism |
| Running without checking working-tree state between phases | Both child-skill checkouts should return to HEAD; verify after Phase C so a leftover detachment doesn't silently break Phase D |
| Declaring the milestone closed without Phase D | Release-readiness is the final gate — the skill's name is "complete-milestone", and the completion certificate is the go/no-go decision, not just "all audits ran" |
