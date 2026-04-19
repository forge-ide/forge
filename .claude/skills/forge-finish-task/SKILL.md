---
name: forge-finish-task
description: Use when implementation is complete and you are ready to submit a Forge GitHub Issue for review — verifies the build, pushes the feature branch, opens a PR, and hands DoD verification and code review OFF to the parent context via a HANDOFF-REQUEST.
---

# forge-finish-task

## Overview

When implementation is done, verify the build locally, push the feature branch, open a PR, and then **hand control back to the parent context** for DoD verification and code review.

**Do not spawn verifier or code-reviewer subagents from inside this skill.** The parent context owns those gates so the verifier's context is guaranteed fresh — independent of the implementer's context. Running them here would collapse that independence and defeat the whole point of fresh-eyes review.

**Do not close the issue** — it closes automatically when the PR merges. **Do not auto-merge the PR** — the parent decides to merge after its gates pass.

## Steps

1. **Read the Definition of Done**

```bash
gh issue view <number> --repo forge-ide/forge
```

Extract every `- [ ]` item verbatim. Do not verify them yet — the parent context will. Record them for the handoff in step 6.

2. **Verify the build locally**

```bash
cargo fmt --check && cargo check --workspace && cargo test --workspace && cargo clippy --all-targets -- -D warnings
```

All must pass before continuing. This is the only verification that happens locally; behavioral DoD verification and code review run in the parent context after handoff (step 6).

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

## Verification status
PR is open; DoD and code-review gates are pending in the parent context (per the forge-finish-task handoff protocol).

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

5. **Update the issue label**

```bash
gh issue edit <number> --remove-label "status: in-progress" --add-label "status: code-review" --repo forge-ide/forge
```

6. **Hand off to the parent context**

Return a structured `HANDOFF-REQUEST` as the final result of this skill. This is the contract the parent context reads to dispatch independent verifier + code-reviewer agents.

**Do not spawn those agents from here.** The parent runs them. If you spawn them inside this skill, the verifier inherits your context (the implementer's context) and the independent-eyes invariant is destroyed silently.

Return the following, verbatim, as the last thing this skill produces:

```
HANDOFF-REQUEST
  pr_url:       https://github.com/forge-ide/forge/pull/<N>
  issue_number: <GitHub issue number>
  branch:       feat/task-<padded-number>
  changed_files:
    <output of `git diff --name-only upstream/main`, one per line>
  dod:
    <paste every DoD checkbox from step 1, verbatim>
  design_note:
    <one paragraph: what you changed and why — enough for a fresh reviewer
     to orient without reading the diff>
```

### What the parent does with the HANDOFF-REQUEST

Documented in `forge-complete-task` Phase 3. Summarized here:

1. Dispatches a fresh-context DoD-verifier agent with the `dod` and `changed_files`.
2. Dispatches a fresh-context `pr-review-toolkit:code-reviewer` (or equivalent) agent with the `pr_url` and `dod`.
3. If either gate surfaces issues, re-engages this skill's parent orchestrator with the findings embedded. The orchestrator returns to Phase 2 (implement), amends the branch, re-pushes, and re-emits HANDOFF-REQUEST.
4. If both gates are clean, merges the PR.

## Protocol for re-engagement after gaps found

If the parent context comes back with DoD or reviewer findings:

1. Return to `forge-complete-task` Phase 2 (implement). Drive the fixes TDD-style: red (test the gap), green (fix), refactor.
2. Re-run step 2 (build verification).
3. Push amended commits to the same branch — do not force-push unless a rebase is genuinely required.
4. Re-emit HANDOFF-REQUEST as the final result.

Do not mark the task complete until the parent confirms the PR merged. Do not auto-merge from inside this skill under any circumstance — the gate results live in the parent context, and merging without them is a silent regression.

## Labels

| Label | Meaning |
|-------|---------|
| `status: in-progress` | Actively being worked on |
| `status: code-review` | PR open, gates running in parent — set here |
| `status: blocked` | Waiting on a dependency |
| `status: complete` | Done — added automatically when PR merges |
| `type: feat` | New feature |
| `type: chore` | Maintenance, scaffolding, docs |
| `type: bug` | Bug fix |

## Common Mistakes

| Mistake | Correct |
|---------|---------|
| Closing the issue manually | Let the PR merge close it via `Closes #N` |
| Pushing to `upstream` instead of `origin` | Always push to your fork (`origin`) |
| Skipping `cargo clippy` | CI enforces `-D warnings` |
| Using the GitHub issue number in the branch name | Use the F-number from the title, zero-padded to 3 digits |
| Spawning a DoD-verifier or code-reviewer subagent from inside this skill | The parent runs those gates; spawning here breaks the independent-eyes invariant |
| Auto-merging the PR from inside this skill | Only the parent merges, after its gates pass |
| Running the DoD verification gate inline here | Document checkboxes in HANDOFF-REQUEST and let the parent verify |
| Omitting the HANDOFF-REQUEST at the end | Without it, the parent can't run gates and the task stalls silently |
| Marking complete before the parent confirms merge | A PR can still fail gates after push — wait for the parent's confirmation |
