---
name: forge-create-task
description: Use when you need to add a new task to the Forge backlog — creates a properly formatted GitHub Issue with the correct milestone, label, and Definition of Done structure.
---

# forge-create-task

## Overview

New tasks are GitHub Issues with a consistent format: `[F-NNN] <title>`, a **Scope** section, and a **Definition of Done** checklist. The F-number matches the GitHub-assigned issue number; since GitHub assigns numbers, capture the assigned number from the create-command output and rewrite the title to match.

**Do not** precompute the next F-number with `gh issue list ... | max + 1`. Issues and PRs share GitHub's number sequence, but `gh issue list` only returns issues — so the formula is systematically wrong whenever any PR has been opened since the last issue, and also stale against any activity between lookup and create.

## Steps

1. **Choose the right milestone and type label** (see tables below)

2. **Create the issue with an `[F-TBD]` placeholder title**

```bash
URL=$(gh issue create \
  --title "[F-TBD] <short imperative title>" \
  --milestone "<Phase N: Title>" \
  --label "type: feat" \
  --repo forge-ide/forge \
  --body "$(cat <<'EOF'
## Scope

<One paragraph: what this ticket builds and why. Reference relevant crates and architecture docs.>

## Definition of Done

- [ ] <Concrete, testable acceptance criterion>
- [ ] <Another criterion>
- [ ] Tests pass in CI
EOF
)")
```

3. **Extract the assigned number and rewrite the title**

```bash
NUM=$(basename "$URL")
gh issue edit "$NUM" --repo forge-ide/forge \
  --title "[F-$NUM] <short imperative title>"
echo "Filed F-$NUM: $URL"
```

### Cross-referencing other issues

If the body needs to reference another issue by F-number, file that issue first so you know its real number. Do not self-reference the issue's own F-number in its body — the body is written before the number is known.

## Issue Format Rules

- Title: `[F-NNN]` prefix, imperative verb, ≤60 chars after the prefix
- Scope: one paragraph — what is built, which crates are touched, why it matters
- DoD: checkbox items must be concrete and verifiable, not vague ("works correctly" is not a DoD item)

## Milestones

| Milestone | Phase |
|-----------|-------|
| Phase 0: Foundations | CLI-only, IPC handshake |
| Phase 1: Single Provider + GUI | Minimal working UI |
| Phase 2: Full Layout + MCP | Agents, full layout |
| Phase 3: Breadth | Multi-provider, skills, containers |
| Phase 4: Polish + v1.0 | Ship-ready |

## Type Labels

| Label | When to use |
|-------|-------------|
| `type: feat` | New user-facing capability or crate behaviour |
| `type: chore` | Scaffolding, config, docs, CI, refactor |
| `type: bug` | Fixing incorrect behaviour |
