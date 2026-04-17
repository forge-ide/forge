---
name: forge-milestone-planner
description: Use when planning a Forge milestone — researches the milestone description, maps its bullet points to discrete GitHub Issues, and creates them with correct F-numbers, Scope, and Definition of Done.
---

# forge-milestone-planner

## Overview

Turns a milestone's outcome statement and bullet list into a set of ready-to-implement GitHub Issues. Each bullet becomes one or more `[F-NNN]` issues with a concrete Scope and verifiable DoD.

## Steps

### 1. Read the milestone

```bash
gh api repos/forge-ide/forge/milestones --jq '.[] | select(.title == "<milestone title>")'
```

Extract: title, description (outcome + bullet list).

### 2. Research — invoke `superpowers:dispatching-parallel-agents`

Two independent research tasks; run them in parallel:

**Agent A — Codebase state:**
> Read the forge repo at /home/jeroche/repos/github/jeff-roche/forge. For each bullet in this milestone's description, find: which crates are touched, what already exists, what is missing. Also read relevant docs under docs/architecture/. Return a per-bullet summary: crate(s), existing symbols, gaps.

**Agent B — Existing issues:**
```bash
gh issue list --repo forge-ide/forge --state all --json number,title \
  --jq 'sort_by(.number)'
```
> Return the full list to find the next available F-number and avoid duplicates.

### 3. Find the next F-number

From Agent B's result:

```bash
gh issue list --repo forge-ide/forge --state all --json title \
  --jq '[.[].title | scan("F-([0-9]+)") | .[0] | tonumber] | max + 1'
```

### 4. Brainstorm the task breakdown — invoke `superpowers:brainstorming`

Before writing any issues, use brainstorming to work through:
- The right granularity (one bullet = one issue? or split/merge?)
- Dependencies between bullets — which must land first?
- Anything implied by the milestone but not explicitly listed
- Correct type label per issue (`feat` / `chore` / `bug`)

The design to confirm: the ordered list of proposed issues with one-line summaries.

**HARD GATE:** Do not write any issues until the task breakdown has been presented and approved.

### 5. Write the issues

For each approved issue, follow this format exactly:

```
Title:  [F-NNN] <imperative verb> <short description>  (≤60 chars after prefix)
Milestone: <Phase N: Title>
Label:  type: feat | type: chore | type: bug

## Scope
<One paragraph: what is built, which crates, which docs/spec sections are relevant.>

## Definition of Done
- [ ] <Concrete, testable criterion — name the symbol, file, or test>
- [ ] <Another criterion>
- [ ] Tests pass in CI
```

**DoD rules:**
- Name the actual symbol, file, or test (not "works correctly")
- Implementation and its tests are separate checkboxes
- CI pass is always the last checkbox

**Label rules:**

| Label | When |
|-------|------|
| `type: feat` | New capability or crate behaviour |
| `type: chore` | Scaffolding, docs, CI, config, refactor |
| `type: bug` | Fixing incorrect behaviour |

### 6. Create issues

Create each issue sequentially (to avoid F-number collisions):

```bash
gh issue create \
  --repo forge-ide/forge \
  --title "[F-NNN] <title>" \
  --milestone "<Phase N: Title>" \
  --label "type: feat" \
  --body "$(cat <<'EOF'
## Scope

<paragraph>

## Definition of Done

- [ ] <criterion>
- [ ] Tests pass in CI
EOF
)"
```

### 7. Verify — invoke `superpowers:verification-before-completion`

After all issues are created, verify before claiming done:

```bash
gh issue list --repo forge-ide/forge \
  --milestone "<Phase N: Title>" --state open \
  --json number,title,labels,milestone
```

Confirm: every planned issue appears with the correct title, milestone, and label. Do not claim the milestone is planned until this evidence is in hand.

## Common Mistakes

| Mistake | Correct |
|---------|---------|
| Skipping brainstorming ("bullets are obvious") | Always brainstorm — granularity and dependencies need explicit review |
| One issue per milestone (too coarse) | One issue per independently deliverable unit |
| Vague DoD: "implement X" | Specific: "`forge_ipc::read_frame` returns `Err` on oversized frame" |
| Generic scope paragraph | Reference actual crate names, file paths, spec sections |
| Creating issues without presenting the plan | Brainstorming hard-gates this |
| Claiming done without checking GitHub | `verification-before-completion` requires evidence |
| Exploring codebase inline | Use `dispatching-parallel-agents` — keep main context surgical |
| Creating issues in parallel | Sequential only — avoids F-number collisions |
