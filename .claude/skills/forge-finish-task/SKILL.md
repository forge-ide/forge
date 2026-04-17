---
name: forge-finish-task
description: Use when implementation is complete and you are ready to submit a Forge GitHub Issue for review — verifies the Definition of Done, pushes the feature branch, and opens a PR to upstream.
---

# forge-finish-task

## Overview

When implementation is done, verify every DoD checkbox, push the feature branch, and open a PR to `forge-ide/forge`. **Do not close the issue** — it closes automatically when the PR merges.

## Steps

1. **View the issue's Definition of Done**

```bash
gh issue view <number> --repo forge-ide/forge
```

Work through every unchecked `- [ ]` item. Do not proceed until all are satisfied.

2. **Verify the build**

```bash
cargo check --workspace && cargo test --workspace && cargo clippy --all-targets -- -D warnings
```

All must pass before continuing.

3. **Create and push the feature branch**

If not already on a feature branch, create one now:

```bash
git checkout -b feat/task-<padded-number>
# e.g. feat/task-002 for issue #4 ([F-002])
```

Use the F-number from the issue title (zero-padded to 3 digits), not the GitHub issue number.

Commit all changes, then push to `origin` (your fork):

```bash
git push -u origin <branch-name>
```

4. **Open a PR to upstream**

```bash
gh pr create \
  --repo forge-ide/forge \
  --base main \
  --head jeff-roche:<branch-name> \
  --title "<issue title>" \
  --body "$(cat <<'EOF'
Closes forge-ide/forge#<issue-number>

## Summary
<bullet points from the DoD>

## Test plan
- `cargo test --workspace` passes
- `cargo clippy --all-targets -- -D warnings` passes

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

5. **Update the issue label**

```bash
gh issue edit <number> --remove-label "status: in-progress" --add-label "status: code-review" --repo forge-ide/forge
```

6. **Confirm**

```bash
gh issue view <number> --repo forge-ide/forge --json state,labels --jq '{state,labels: .labels | map(.name)}'
```

Expected: `"state": "OPEN"` with `status: code-review` label.

## Labels

| Label | Meaning |
|-------|---------|
| `status: in-progress` | Actively being worked on |
| `status: code-review` | PR open, awaiting review — set here |
| `status: blocked` | Waiting on a dependency |
| `status: complete` | Done — added automatically when PR merges |
| `type: feat` | New feature |
| `type: chore` | Maintenance, scaffolding, docs |
| `type: bug` | Bug fix |

## Common Mistakes

- Closing the issue manually — let the PR merge close it via "Closes #N"
- Pushing to `upstream` instead of `origin` — always push to your fork (`origin`)
- Skipping `cargo clippy` — CI enforces `-D warnings`
- Using the GitHub issue number in the branch name instead of the F-number from the title
