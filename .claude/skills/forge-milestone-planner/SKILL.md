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

### 2. Research via subagents — do not explore inline

Spawn two subagents in parallel:

**Subagent A — Codebase state:**
> Read the forge repo at /home/jeroche/repos/github/jeff-roche/forge. For each bullet in this milestone's description, find: which crates are touched, what already exists, what is missing. Also read relevant docs under docs/architecture/. Return a per-bullet summary: crate(s), existing symbols, gaps.

**Subagent B — Existing issues:**
```bash
gh issue list --repo forge-ide/forge --state all --json number,title --jq 'sort_by(.number)'
```
> Return the full list so we can find the next available F-number and avoid duplicates.

### 3. Find the next F-number

From Subagent B's result:
```bash
gh issue list --repo forge-ide/forge --state all --json title \
  --jq '[.[].title | scan("F-([0-9]+)") | .[0] | tonumber] | max + 1'
```

### 4. Map bullets → issues

For each bullet in the milestone description:
- If the bullet is a single cohesive unit of work → one issue
- If it contains multiple independently deliverable pieces → split into multiple issues
- If it depends on another bullet being done first → note the dependency in Scope

Use the codebase research (Subagent A) to write specific, accurate Scope paragraphs.

**Issue format:**
```
Title: [F-NNN] <imperative verb> <short description>  (≤60 chars after prefix)
Milestone: <Phase N: Title>
Label: type: feat | type: chore | type: bug
Body:
  ## Scope
  <One paragraph: what is built, which crates, which docs/specs are relevant. Be specific.>

  ## Definition of Done
  - [ ] <Concrete, testable criterion — name the symbol, file, or test>
  - [ ] <Another criterion>
  - [ ] Tests pass in CI
```

**DoD rules:**
- Name the actual symbol, file, or test (not "works correctly")
- Integration/unit tests are a separate checkbox from the implementation
- CI pass is always the last checkbox

**Label rules:**

| Label | When |
|-------|------|
| `type: feat` | New capability or crate behaviour |
| `type: chore` | Scaffolding, docs, CI, config, refactor |
| `type: bug` | Fixing incorrect behaviour |

### 5. Present the plan before creating

Output a numbered list:
```
[F-NNN] <title> (type: feat)
  Scope: <one sentence>
  DoD: <bullet count> items

[F-NNN+1] <title> (type: chore)
  ...
```

**Stop here.** Ask for confirmation before creating any issues.

### 6. Create issues

After confirmation, create each issue with `forge-create-task` conventions:

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

Create sequentially (each issue must exist before the next to avoid F-number collisions).

## Common Mistakes

| Mistake | Correct |
|---------|---------|
| One issue per milestone (too coarse) | One issue per independently deliverable unit |
| Vague DoD: "implement X" | Specific: "`forge_ipc::read_frame` returns `Err` on oversized frame" |
| Generic scope paragraph | Reference actual crate names, file paths, and spec sections |
| Creating issues without confirming | Always present plan and wait for approval |
| Exploring codebase inline | Spawn Explore subagent — keep main context surgical |
| Skipping dependency notes | If bullet B needs bullet A, say so in Scope |
