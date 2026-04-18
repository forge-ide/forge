---
name: forge-milestone-uat-author
description: Use when authoring the User Acceptance Test plan for a Forge milestone — clusters the milestone's closed-issue Definitions of Done into user-observable outcomes, drafts docs/testing/phase-N-uat.md, phase-N-uat.sh, and phase-N-uat-setup.md in the established house style (matching phase0 and phase1), verifies every DoD checkbox is either covered by a UAT scenario, deferred to unit tests with rationale, or honestly flagged as a known gap, and opens a PR. Trigger on phrases like "write UAT for Phase N", "author the milestone UAT plan", "draft phase-N-uat.md", "we need UAT tests for this phase", or any milestone UAT authorship task. This is the only milestone-level skill that produces source files (committed via PR) rather than GitHub issues.
---

# forge-milestone-uat-author

## Overview

Authors the three UAT files for a Forge milestone:
- `docs/testing/phase<N>-uat.md` — the plan
- `docs/testing/phase<N>-uat.sh` — the harness
- `docs/testing/phase<N>-uat-setup.md` — the one-time setup doc

**Structurally different from the seven audit siblings**: the audits produce GitHub *issues*; this skill produces committed *files*. Output is a **PR**, not issues. Everything else (scope gathering → brainstorming → hard gates → verification) mirrors the audit spine.

## The philosophy of a good UAT plan

Read this before doing anything else — it is the load-bearing framing that separates a useful UAT plan from a useless one. Prior phases encoded this in `docs/testing/phase1-uat.md`'s "What this plan covers" section; the skill must honor it.

1. **A UAT verifies user-observable behavior that depends on multiple tickets working together.** It is not a restatement of each ticket's DoD. Individual DoDs are enforced by unit tests; UAT is the integration gate.
2. **Cluster DoDs into outcomes.** A milestone with 14 tickets does not need 14 UATs. It needs 6–10 UATs, each exercising a chain of tickets through a user-observable flow.
3. **Backend primitives with no user surface are deferred to unit tests**, with a dedicated section at the bottom of the plan that names them and explains why. This is not a dodge — it is a contract that keeps the UAT plan focused.
4. **Be honest about shippable vs. claimed.** If the milestone description says X but the code ships Y, the plan must flag the gap explicitly (`phase1-uat.md` does this with its "Known gap" section and the UAT-01c Blocked variant). Papering over the delta produces a UAT plan that passes while the actual feature is broken.
5. **The outcome gate comes first.** UAT-01 (or UAT-01a) is the milestone's headline capability, written as a single end-to-end flow. If it passes, the milestone's top-level promise was shipped.

## Arguments

| Argument | Form | Meaning |
|----------|------|---------|
| Milestone (required) | `"Phase N: Title"` | The GitHub milestone title to author UAT for |

If the argument is missing, list candidate milestones and ask:

```bash
gh api repos/forge-ide/forge/milestones --jq '.[] | {title, state, open_issues, closed_issues}'
```

## Steps

### 1. Gather scope and house style — invoke `Explore`

Delegate to an `Explore` subagent. Brief:

> For milestone `"<milestone>"`:
>
> 1. Fetch milestone metadata: `gh api repos/forge-ide/forge/milestones --jq '.[] | select(.title == "<milestone>")'`. Return title, description (the **outcome statement** + bullet list), and state.
> 2. List the milestone's **closed issues** (each one is a ticket that shipped): `gh issue list --repo forge-ide/forge --milestone "<milestone>" --state closed --json number,title,body,labels --limit 200`. Return the list.
> 3. For **every closed issue**, extract the Definition of Done checkboxes from the issue body. Return the full DoD set — one entry per (issue number, checkbox text) pair. This is what coverage will be verified against.
> 4. List the milestone's **merged PRs**: `gh pr list --repo forge-ide/forge --search 'milestone:"<milestone>" is:merged' --json number,title,body,mergedAt --limit 200`. Return titles, bodies (look for "Known gap" or "Not wired" language), and merge dates.
> 5. **Read prior-phase UAT files for house style** — whichever exist in `docs/testing/`:
>    - `phase0-uat.md`, `phase0-uat.sh`
>    - `phase1-uat.md`, `phase1-uat.sh`, `phase1-uat-setup.md`
>    - etc.
>    Return a summary of the conventions you observed: section ordering, per-UAT structure, scope labeling (F-numbers), vehicle declarations, preparation-block format, steps-table format, failure-criteria format, shell-script flag set, PASS/FAIL/SKIP coloring, setup-doc section structure.
> 6. Identify **shippable-vs-claimed deltas**: milestone description bullets that do not correspond to a closed issue, or PRs whose body flags a "not wired" condition. Return these as candidate "Known gap" items.
> 7. Determine the phase number (`<N>`) for the filenames.
>
> Return: milestone facts, full DoD set with (issue, checkbox) provenance, PR list, house-style summary, candidate known-gap list, phase number.

### 2. Derive the scenario list — invoke `superpowers:brainstorming`

The load-bearing reason this is brainstormed and not mechanical: **clustering DoDs into outcomes** is exactly the interpretive step where a machine-generated plan goes wrong. Dumping one UAT per DoD is easy and useless. A good plan collapses ten DoDs into three outcomes, explicitly defers the four backend DoDs that have no user surface, and names the one scenario that is Blocked because the milestone shipped less than it claimed.

Work through with the user, seeded by the Phase 1 findings:

- **The outcome gate** — what is the milestone's headline capability? Which DoDs combine to deliver it? This is UAT-01 (or UAT-01a if variants are needed).
- **Secondary user-observable outcomes** — for each remaining user-facing flow, identify the DoDs it exercises and draft a one-line scenario summary. Aim for 5–10 total UATs, not 14+.
- **Variants (a/b/c...)** — when an outcome has a reachable path (e.g. mock provider) and a blocked path (e.g. real Ollama not yet wired), both get entries; the blocked one is marked Blocked with a forward-looking note.
- **Backend primitives to defer to unit tests** — DoDs with no user surface. Name each, explain why it has no UAT, and say which crate's tests cover it.
- **Known gaps** — any milestone bullet without a shipping issue, or any PR that did less than its title implies. These appear in the plan's "Known gap" section.
- **Automation vehicle per UAT** — GUI (Playwright + tauri-driver), disk/state (bash), or hybrid. Match prior-phase patterns.

Output: an **outcome matrix** with columns: `UAT-ID | title | covered DoD checkboxes | vehicle | variant | status`.

**HARD GATE:** Do not begin Step 3 until the outcome matrix has been presented and approved. This is the point where a bad cluster is cheap to fix; after drafting, it is expensive.

### 3. If UI is in scope — invoke `frontend-design:frontend-design`

Any UAT whose vehicle is Playwright exercises a UI flow. For each such UAT, invoke `frontend-design:frontend-design` to surface:

- The expected visual/interaction states the flow should exhibit (loading, empty, error, success)
- The specific selectors or semantic landmarks the UAT should assert against (match the design system's conventions, not ad-hoc IDs)
- Any accessibility expectations the UAT should verify (keyboard operability, focus management)

These feed into the Preparation and Steps blocks of the affected UATs.

**Skip this step entirely** if the milestone has no UI-exercising UATs.

### 4. Draft `phase<N>-uat.md` — main context

Match the house-style summary from Phase 1. The structure (verbatim from `docs/testing/phase1-uat.md`):

```markdown
# Phase <N> User Acceptance Test Plan

**Scope:** <milestone title> — <one-line summary of the capability set>.
**Outcome gate:** <one-to-two-sentence statement of the milestone's headline capability, restricted to what actually shipped>.

---

## Known gap before reading further          ← include only if there are known gaps

<Paragraph honestly describing shippable-vs-claimed deltas, with file/line references where applicable. This section is load-bearing; do not soften it.>

---

## What this plan covers

<Paragraph explaining the philosophy: UATs verify user-observable behavior that depends on multiple tickets, not each DoD individually. Name the count of tickets shipped vs. the count of UATs.>

Automation vehicle:
- **GUI UATs** — Playwright driving the Tauri app via `tauri-driver` [...]
- **Disk / state UATs** — bash harness invoking `forge` / `forged` [...]

---

## Prerequisites

<Table: Item | Requirement — match the phase1 shape. Include Ollama / mock-provider rows only if applicable.>

---

## UAT-01[a]: Outcome gate — <one-line title>

**Scope:** <F-NN + F-NN + ... covered DoDs>
**Vehicle:** <Playwright+tauri-driver | bash | hybrid>
**Why this test exists:** <one line — this is the milestone's outcome gate restricted to what shipped>

<Preparation block with mock scripts / fixtures / env vars, matching phase1's fenced-bash style.>

| Step | Action | Expected |
|------|--------|----------|
| 1 | ... | ... |
| N | ... | ... |

**Failure criteria:** <one line — enumerate the specific observations that would fail this UAT>

---

## UAT-02: <title>

<same shape as above>

... continue for each outcome ...

---

## Backend primitives covered by unit tests, not UAT

<List each deferred DoD with: symbol/crate, covering unit test file, one-line rationale for why it has no user surface.>

---

## Known-gap follow-up

<If there were known gaps: list the forward-looking actions — e.g. "UAT-0Nc unblocks when F-XXX lands; either a Phase <N> cleanup ticket or a Phase <N+1> item to triage before declaring this milestone complete".>
```

Draft inline; do not delegate. Generative writing benefits from full context over the whole plan.

### 5. Draft `phase<N>-uat.sh` — main context

Match the phase1 harness shape (read it first if memory is not fresh). Required elements:

- Shebang + header comment with usage, flags, prerequisites pointer
- `set -euo pipefail`
- `REPO_ROOT` detection via `BASH_SOURCE`
- Color helpers (RED/GREEN/YELLOW/BOLD/RESET) and `pass`/`fail`/`skip`/`header` functions
- PASS/FAIL/SKIP counters and `FAILED_TESTS` array
- Argument parsing — at minimum `--build`, `--test UAT-NN`, and the gui-only / cli-only split if the plan has both vehicles
- One function per UAT (`uat_01a`, `uat_02`, …). Each function wraps either a Playwright invocation (`pnpm --filter app exec playwright test ...`) or bash-driven `forge`/`forged` commands plus filesystem assertions
- A final summary block printing PASS / FAIL / SKIP counts and the failed-test list, returning non-zero on FAIL > 0

Wire the GUI UATs' Playwright specs under `web/packages/app/tests/phase<N>/` — note this in the plan's Prerequisites table.

### 6. Draft `phase<N>-uat-setup.md` — main context

Match the phase1 setup doc shape. Numbered sections for each setup class that appears in Prerequisites. Typical sections (include only those that apply):

1. Rust binaries — `cargo build --workspace`, note which binaries live where
2. pnpm workspace — install, build, `check-tokens`
3. Playwright + tauri-driver install
4. Ollama (if applicable)
5. Mock provider + fixture placement
6. Scratch workspace convention

Each section: short prose explaining what the user is doing + the exact commands.

### 7. Coverage verification — HARD GATE — invoke a subagent

Delegate to a fresh general-purpose subagent. Brief:

> Given:
> - The full DoD checkbox set for milestone `"<milestone>"` — one entry per (issue number, checkbox text) — attached as `dod.json`
> - The draft `phase<N>-uat.md` — attached
> - The outcome matrix from Step 2 — attached
>
> For **every** DoD checkbox, return a row classifying it as:
> - `covered` — a UAT scenario in the plan exercises it; name the UAT-ID
> - `deferred-unit-test` — the plan's "Backend primitives" section names it with rationale
> - `known-gap` — the plan's "Known gap" section honestly flags that this DoD did not ship as described
> - `unmapped` — neither of the above
>
> Also return the reverse check: for every UAT scenario in the plan, list the DoDs it claims to cover, and whether each claimed DoD is actually in the DoD set.
>
> Output: two tables — `dod → classification + UAT-ID | section | status`, and `UAT → claimed DoDs (valid | invalid)`.

**HARD GATE:** If any DoD is `unmapped`, or any UAT claims a DoD not in the set, fix the drafts and re-run the verification. Do not proceed until the mapping is clean.

### 8. Self-review against prior phase — optional subagent

Delegate to an `Explore` subagent (or skip if this is Phase 0 / no prior phase exists):

> Compare the three new files (`phase<N>-uat.md`, `phase<N>-uat.sh`, `phase<N>-uat-setup.md`) to the matching prior-phase files. Flag:
> - Section ordering that diverges without cause
> - Per-UAT field ordering that diverges (Scope / Vehicle / Why / Preparation / Steps / Failure criteria)
> - Harness flag set that diverges — new plans should be a superset or match, not a subset
> - Shell function naming conventions (snake_case `uat_NNx`) that diverge
>
> Return: a short drift list, or "no material drift".

Address any drift flagged.

### 9. Create the branch, commit, and open the PR

Branch + commit from the main context:

```bash
git checkout -b "docs/phase<N>-uat"
git add docs/testing/phase<N>-uat.md \
        docs/testing/phase<N>-uat.sh \
        docs/testing/phase<N>-uat-setup.md
# If new Playwright specs were stubbed out, add them too:
# git add web/packages/app/tests/phase<N>/

git commit -m "$(cat <<'EOF'
docs(testing): add Phase <N> UAT plan, harness, and setup

Covers milestone "<milestone>": <N> UATs across <vehicle mix>.
Outcome gate: UAT-01 (<one-line>).
Backend primitives deferred to unit tests are listed in the plan.
<If applicable:> Known gap: <one-line> (see plan for details).
EOF
)"

git push -u origin "docs/phase<N>-uat"

gh pr create \
  --repo forge-ide/forge \
  --base main \
  --title "docs(testing): Phase <N> UAT plan, harness, and setup" \
  --milestone "<milestone>" \
  --label "type: chore" \
  --body "$(cat <<'EOF'
## Summary

Adds the Phase <N> UAT trio:
- `docs/testing/phase<N>-uat.md` — plan
- `docs/testing/phase<N>-uat.sh` — harness
- `docs/testing/phase<N>-uat-setup.md` — one-time setup

<Short paragraph on vehicle mix and outcome gate.>

## Coverage

<Paste the `dod → UAT-ID | section` table from Step 7 verbatim. Reviewers can scan it to confirm every DoD is accounted for.>

## Known gaps flagged in the plan

<Bulleted list, or "none".>

## Reviewer checklist

- [ ] Every closed issue's DoD appears in the coverage table
- [ ] Outcome gate (UAT-01[a]) matches the milestone's headline capability
- [ ] Backend-primitive deferrals each name a covering unit test
- [ ] Known gaps are honestly described (not softened)
- [ ] Harness flags match the prior phase's flag set
EOF
)"
```

### 10. Verify — invoke `superpowers:verification-before-completion`

```bash
gh pr list --repo forge-ide/forge --head "docs/phase<N>-uat" --json number,title,labels,milestone
```

Confirm:
- The PR exists
- All three files are in the diff
- The PR body contains the full coverage table from Step 7
- The milestone and label are set

Do not claim done until this evidence is in hand.

## Delegation rules

| Work type | Where it runs |
|-----------|---------------|
| Milestone metadata, DoD extraction, prior-phase style read, known-gap detection | `Explore` subagent |
| Outcome matrix / DoD clustering | `superpowers:brainstorming` with the user |
| UI-flow / state expectations for Playwright-vehicle UATs | `frontend-design:frontend-design` |
| Drafting the three files | Main context (generative writing benefits from full context) |
| DoD coverage verification | Fresh general-purpose subagent |
| Style-drift check vs. prior phase | `Explore` subagent (optional) |
| Git branch + commit + PR open | Main context, sequential |
| Final verification | `superpowers:verification-before-completion` |

## Common mistakes

| Mistake | Correct |
|---------|---------|
| Generating one UAT per DoD | Cluster DoDs into user-observable outcomes; a 14-ticket milestone should produce ~6–10 UATs, not 14 |
| Skipping the deferred-unit-test section | Every DoD without a user surface must be explicitly named and paired with its unit-test location — silent omission is a bug |
| Softening shippable-vs-claimed gaps | "Known gap" sections are load-bearing; phase1 set the precedent — match its honesty |
| Generic UAT steps ("verify it works") | Every step must name a specific observation (selector, file content, stdout line) that distinguishes pass from fail |
| Inventing a new harness flag set | The `.sh` file's flag set should match or be a superset of the prior phase's — contributors shouldn't re-learn the CLI each phase |
| Wiring GUI UATs without Playwright selectors grounded in the design system | Use semantic landmarks / data attributes the design system already exposes, not ad-hoc IDs |
| Skipping coverage verification because "it looks complete" | The hard gate exists because unmapped DoDs escape notice; evidence before assertions |
| Writing UATs for features that did not ship | If a milestone bullet has no closed issue, the UAT goes in the Known-gap section, not the main plan |
| Opening issues instead of a PR | This skill's output is a PR with committed files — a structural departure from the audit siblings; don't reach for `gh issue create` |
| Forgetting to link Playwright specs under `web/packages/app/tests/phase<N>/` | If the plan cites Playwright, the specs directory should be created (even if stubbed) so the harness has somewhere to point |
| Claiming done without `gh pr list` evidence | `verification-before-completion` requires it |
