---
name: forge-start-task
description: Use when picking up a Forge task to work on — lists open issues, claims one by adding the in-progress label, and loads its context before starting implementation.
---

# forge-start-task

## Overview

Before starting any implementation work, sync main with upstream, claim a GitHub Issue, create a feature branch, and understand its Definition of Done. This prevents duplicate work and keeps status visible.

## Steps

1. **Sync main with upstream**

```bash
git fetch upstream
git checkout main
git merge --ff-only upstream/main
git push origin main
```

If `--ff-only` fails (local commits on main), stop and resolve with the user before continuing.

2. **List open tasks in the current phase**

```bash
gh issue list --repo forge-ide/forge --milestone "Phase 0: Foundations" --label "" --state open --json number,title,labels --jq '.[] | "\(.number) \(.title) [\(.labels | map(.name) | join(", "))]"'
```

Adjust `--milestone` to the active phase. Omit `--label ""` to see all open issues.

3. **Pick an issue** — prefer no `status:` label (unclaimed). Avoid anything already `status: in-progress`.

4. **Mark it in-progress**

```bash
gh issue edit <number> --add-label "status: in-progress" --repo forge-ide/forge
```

5. **Create a feature branch off main**

Use the F-number from the issue title (zero-padded to 3 digits), not the GitHub issue number:

```bash
git checkout -b feat/task-<padded-f-number>
# e.g. [F-003] → feat/task-003
```

6. **Read the issue body**

```bash
gh issue view <number> --repo forge-ide/forge
```

Note the **Scope** and **Definition of Done** checkboxes — these are the acceptance criteria.

## Labels

| Label | Meaning |
|-------|---------|
| `status: in-progress` | Actively being worked on — add when you claim |
| `status: code-review` | PR open, awaiting review |
| `status: blocked` | Waiting on a dependency — use instead of in-progress |
| `status: complete` | Done — added automatically when PR merges |
| `type: feat` | New feature |
| `type: chore` | Maintenance, scaffolding, docs |
| `type: bug` | Bug fix |

## Milestones

| Milestone | Phase |
|-----------|-------|
| Phase 0: Foundations | CLI-only, IPC handshake |
| Phase 1: Single Provider + GUI | Minimal working UI |
| Phase 2: Full Layout + MCP | Agents, full layout |
| Phase 3: Breadth | Multi-provider, skills, containers |
| Phase 4: Polish + v1.0 | Ship-ready |
