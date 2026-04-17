---
name: forge-finish-task
description: Use when implementation is complete and you are ready to submit a Forge GitHub Issue for review — verifies the Definition of Done, pushes the feature branch, and opens a PR to upstream.
---

# forge-finish-task

## Overview

When implementation is done, verify every DoD checkbox, push the feature branch, and open a PR to `forge-ide/forge`. **Do not close the issue** — it closes automatically when the PR merges.

## Steps

1. **Read the Definition of Done**

```bash
gh issue view <number> --repo forge-ide/forge
```

Extract every `- [ ]` item. You will verify each one in the next step.

2. **Verify each DoD item via a fresh-context subagent**

Spawn a general-purpose subagent. Give it the DoD checkboxes and nothing else — no implementation context, no file names, no hints. It must find evidence independently.

```
Prompt template:
  Verify the following Definition of Done checkboxes for forge repo at /home/jeroche/repos/github/jeff-roche/forge.
  For each item, find concrete code evidence (grep result, file read, or test run).
  Do NOT accept "it compiles" as evidence for a specific item — find the actual symbol, method, or file.

  Definition of Done:
  <paste DoD checkboxes>

  For each checkbox report:
  - VERIFIED: <what you found and where>
  - NOT FOUND: <what is missing>

  If any item is NOT FOUND, stop and report — do not guess.
```

**If any item comes back NOT FOUND:** implement the missing items, then spawn a fresh-context DoD subagent and repeat this step. Loop until every checkbox comes back VERIFIED. Each iteration must use a new subagent with no memory of prior runs.

3. **Verify the build**

```bash
cargo fmt --check && cargo check --workspace && cargo test --workspace && cargo clippy --all-targets -- -D warnings
```

All must pass before continuing.

4. **Code review with a fresh-context subagent**

Spawn a `feature-dev:code-reviewer` subagent. Give it the issue number, the DoD, and the list of changed files — nothing else. It must derive its own understanding from the code.

```
Prompt template:
  Review the implementation for forge-ide/forge issue #<N>: "<issue title>"

  Definition of Done:
  <paste DoD checkboxes>

  Changed files (git diff --name-only upstream/main):
  <paste list>

  Verify:
  1. Every DoD item is fully implemented — not just structurally present but behaviorally correct.
  2. No regressions in existing tests or public APIs.
  3. Rust-specific issues: missing derives, incorrect serde attributes, unused imports, clippy violations.
  4. Report only HIGH-confidence findings. Skip style nitpicks.
```

**If the reviewer raises any HIGH-confidence finding:** fix all findings, then spawn a fresh-context code-reviewer subagent and repeat this step. Loop until the reviewer reports no HIGH-confidence findings. Each iteration must use a new subagent with no memory of prior runs. Do not push code with known issues.

5. **Create and push the feature branch**

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

6. **Open a PR to upstream**

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

7. **Update the issue label**

```bash
gh issue edit <number> --remove-label "status: in-progress" --add-label "status: code-review" --repo forge-ide/forge
```

8. **Confirm**

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
- Checking a DoD box without verifying the code — a passing test or a `pub use` in `lib.rs` is evidence; memory is not
- Assuming a prior issue is fully implemented without running the verification grep — always confirm types/methods exist before marking dependents as unblocked
