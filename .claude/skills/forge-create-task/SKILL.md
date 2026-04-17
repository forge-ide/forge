---
name: forge-create-task
description: Use when you need to add a new task to the Forge backlog — creates a properly formatted GitHub Issue with the correct milestone, label, and Definition of Done structure.
---

# forge-create-task

## Overview

New tasks are GitHub Issues with a consistent format: `[F-NNN] <title>`, a **Scope** section, and a **Definition of Done** checklist. Use the next available F-number.

## Steps

1. **Find the next issue number**

```bash
gh issue list --repo forge-ide/forge --state all --json number --jq '[.[].number] | max + 1'
```

2. **Choose the right milestone and type label** (see tables below)

3. **Create the issue**

```bash
gh issue create \
  --title "[F-NNN] <short imperative title>" \
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
)"
```

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
