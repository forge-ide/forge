---
name: forge-close-milestone
description: Use when actually shipping a Forge milestone after forge-milestone-release-readiness has returned GO (or GO WITH WAIVERS) — bumps the workspace and package versions to the release version, commits and tags, pushes, creates the GitHub release with notes pulled from CHANGELOG.md, closes the GitHub milestone, closes the consolidated audit-report issues, and then bumps main to the next dev cycle. Refuses to run without a GO decision on record. Every action is idempotent so a partial run can be safely resumed. Trigger on phrases like "close out and ship Phase N", "cut the release for this milestone", "tag and release the milestone", "we got a GO — ship it", or any actual release-cut invocation.
---

# forge-close-milestone

## Overview

The **execution skill** that ships a milestone after the gauntlet has returned GO. This is the only milestone skill that performs destructive / irreversible actions (tag push, `gh release create`, milestone close) — so it gates heavily, builds a manifest the user confirms before any state change, and makes every action idempotent.

Ordering context in the skill family:
1. `forge-complete-milestone` runs the audit gauntlet and ends with `forge-milestone-release-readiness` returning a decision.
2. If the decision is **GO** or **GO WITH WAIVERS**, *this* skill ships it.
3. If the decision is **NO-GO**, this skill refuses to run until the blockers are cleared and release-readiness is re-run.

Not in scope: creating the next milestone, announcements, changelog authorship (the release-readiness skill already verified the changelog is complete).

## Arguments

| Argument | Form | Meaning |
|----------|------|---------|
| Milestone (required) | `"Phase N: Title"` | The GitHub milestone to close and release |

## Phase 0: Precondition checks — fail fast

These are all hard refusals — do not proceed past any failure.

```bash
# 1. Fetch the release-readiness report for this milestone
gh issue list --repo forge-ide/forge --milestone "<milestone>" \
  --search 'label:"release: audit"' --json number,title,state,body --limit 1
```

From that report body, confirm the **Decision** line reads `GO` or `GO WITH WAIVERS`. If it reads `NO-GO` or the report does not exist: stop and tell the user to run `forge-complete-milestone` (or `forge-milestone-release-readiness` directly) first.

```bash
# 2. Working tree must be clean
git status --short
# 3. Must be on main, up to date with origin/main
git rev-parse --abbrev-ref HEAD
git fetch origin main
git rev-list --count HEAD..origin/main     # must be 0
git rev-list --count origin/main..HEAD     # must be 0
```

Any mismatch → stop. Do **not** auto-checkout, auto-pull, or auto-rebase — surface the divergence and let the user settle it.

## Phase 1: Build the release manifest — invoke `Explore`

Delegate to an `Explore` subagent. Brief:

> For milestone `"<milestone>"`, build a release manifest.
>
> 1. **Current versions**: read `[workspace.package].version` from the root `Cargo.toml`. Then find every `package.json` under `web/` (including `web/package.json` and every `web/packages/*/package.json`) and return each file's `version`. Return the full list: `(path, current_version)`.
> 2. **Proposed release version**: the release-readiness report's Step 2 criteria recorded the expected version-bump regime (major/minor/patch/none). Read that from the report body. If it says "no bump needed", the release version equals the current version. Otherwise, propose the bumped value (e.g. current `0.2.0-dev` → release `0.2.0`; current `0.2.0` → release `0.3.0` if the criteria say minor). Return the proposed value.
> 3. **Tag name**: look at existing git tags to derive the convention: `git tag -l --sort=-v:refname | head -5`. Propose a tag name that matches — usually `v<release_version>`. If there's a phase-marker convention (e.g. `v0.2.0-phase2`), match it.
> 4. **Post-release dev version**: propose the next dev value (e.g. release `0.2.0` → `0.3.0-dev`, or `0.2.1-dev`, depending on the repo's precedent from tag history). If the repo has never used a `-dev` suffix, propose the same-version continuation and note it.
> 5. **Release notes source**: read the CHANGELOG.md section for the release version. Extract it verbatim (that's the release-note body). If the section is missing or empty, flag it — release-readiness is supposed to have caught this, so surface the discrepancy.
> 6. **Milestone number**: from `gh api repos/forge-ide/forge/milestones`.
> 7. **Close-list of consolidated audit report issues** under this milestone: `gh issue list --milestone "<milestone>" --search 'label:"security: audit" OR label:"docs: audit" OR label:"quality: audit" OR label:"frontend: audit" OR label:"perf: audit" OR label:"compat: audit" OR label:"release: audit"' --state open --json number,title,labels`.
> 8. **Open finding issues** under this milestone (i.e. non-audit issues still open): `gh issue list --milestone "<milestone>" --state open --json number,title,labels`. These are deferred findings that will NOT be closed by this skill; they're candidates for roll-over to the next milestone.
>
> Return: manifest as a single structured object.

## Phase 2: HARD GATE — present manifest and confirm

Display the full manifest in a single block, ordered by the execution sequence. Example shape (substitute real values):

```
Release manifest for <milestone>

Version bumps — release:
  Cargo.toml (workspace)    0.2.0-dev  →  0.2.0
  web/package.json          0.0.0      →  0.2.0
  web/packages/app/         0.0.0      →  0.2.0
  web/packages/design/      0.0.0      →  0.2.0
  web/packages/ipc/         0.0.0      →  0.2.0

Git operations:
  Commit:  chore(release): v0.2.0
  Tag:     v0.2.0  (annotated)
  Push:    origin main + origin v0.2.0

GitHub release:
  Title:   Phase 2: Full Layout + MCP — v0.2.0
  Tag:     v0.2.0
  Notes:   <first 10 lines of CHANGELOG section, truncated with "…">

GitHub milestone:
  Close:   Phase 2: Full Layout + MCP  (#NN)

Audit-report issues to close:
  #123  Security audit report: Phase 2
  #124  Quality review report: Phase 2
  #125  Release readiness report: Phase 2
  ... (N total)

Deferred findings to stay open (will NOT be closed by this skill):
  #130  [perf: low] ...
  #131  [docs: medium] ...
  ... (M total)

Version bumps — post-release (second commit, to resume dev on main):
  Cargo.toml (workspace)    0.2.0  →  0.3.0-dev
  web/package.json          0.2.0  →  0.3.0-dev
  ... (same paths)

Commit:  chore(release): bump to v0.3.0-dev
Push:    origin main
```

Explicitly ask: **"Close and release <milestone> using this manifest? Type 'yes' to proceed, or say which values to change."**

Do not proceed without a `yes`. If the user edits any value, redisplay the full manifest and ask again — iterations are cheap; a wrong tag is not.

**Hard rule:** never take a `yes` implied from context. A distinct confirmation turn is required because the next step pushes to a shared remote.

## Phase 3: Execute the release

Each sub-step checks its done-state first and skips cleanly if already applied. This is what makes the skill re-runnable after a partial failure.

### 3a. Release-version bump

For each `(path, current, proposed)` in the manifest's release-bump list:

```bash
# Check if already at proposed version (idempotence)
current_in_file=$(<parse the file at path>)
if [ "$current_in_file" = "<proposed>" ]; then
  echo "skip: $path already at <proposed>"
else
  # Apply the edit
  <sed/edit the version field in the file>
fi
```

Use the `Edit` tool for each file, not bash `sed`, so the change is reviewable inline.

### 3b. Release commit

```bash
# Idempotence: if the top commit is already "chore(release): v<version>", skip
git log -1 --pretty=%s
# Otherwise:
git add <listed files>
git commit -m "chore(release): v<version>"
```

### 3c. Tag the release

```bash
# Idempotence: check if tag exists
git tag -l "<tag>"
# If empty, create:
git tag -a "<tag>" -m "Release <tag>: <milestone>"
```

### 3d. Push commit and tag

```bash
git push origin main
git push origin "<tag>"
```

After this point the release is **public** — cannot be cleanly undone. If the user interrupts here, re-running the skill will detect the tag and skip to 3e.

### 3e. Create the GitHub release

```bash
# Idempotence: check if release exists for this tag
gh release view "<tag>" --repo forge-ide/forge >/dev/null 2>&1
# If missing, create:
gh release create "<tag>" \
  --repo forge-ide/forge \
  --title "<title>" \
  --notes "$(cat <<'EOF'
<CHANGELOG section, verbatim>
EOF
)"
```

### 3f. Close the GitHub milestone

```bash
# Idempotence: check state
gh api "repos/forge-ide/forge/milestones/<N>" --jq .state
# If "open":
gh api --method PATCH "repos/forge-ide/forge/milestones/<N>" -f state=closed
```

### 3g. Close the consolidated audit-report issues

For each report issue in the close-list, if state is `open`:

```bash
gh issue close <number> --repo forge-ide/forge \
  --reason completed \
  --comment "Closed by forge-close-milestone for $(git describe --tags --exact-match)."
```

**Do not close the open-findings list** — those are deferred issues that roll forward to be triaged into the next milestone.

### 3h. Offer roll-over for deferred findings

Ask the user: "There are M open findings that were filed during this milestone. Move them to the next milestone now, leave them unassigned, or decide later?" Three options:

- **Move**: require the user to name the next milestone title; apply via `gh issue edit --milestone "<next>"` per issue
- **Unassign**: `gh issue edit --remove-milestone` per issue (they become backlog)
- **Later**: do nothing; surface the list in the Phase 5 summary so the user can handle it separately

Never default — always ask.

## Phase 4: Post-release dev-version bump

This bumps main back onto a dev track so continuing work isn't accidentally on the release version.

### 4a. Post-release version edits

Same idempotence pattern as 3a — check current, skip if already at the dev value.

### 4b. Post-release commit

```bash
git add <files>
git commit -m "chore(release): bump to v<dev-version>"
```

### 4c. Push

```bash
git push origin main
```

**If the user elected no post-release bump** (they can override this in Phase 2), skip Phase 4 entirely.

## Phase 5: Summary

Print a single-screen summary with direct links:

- **Release**: `gh release view <tag> --json url --jq .url`
- **Tag**: `https://github.com/forge-ide/forge/releases/tag/<tag>`
- **Milestone**: `https://github.com/forge-ide/forge/milestone/<N>?closed=1`
- **HEAD**: `git log -1 --pretty='%h %s'` (should be the post-release dev-bump commit)
- **Closed report issues**: list
- **Deferred findings**: list with their roll-over fate (moved / unassigned / pending)

## Delegation rules

| Work type | Where it runs |
|-----------|---------------|
| GO-decision verification, clean-tree verification | Main context (Bash, cheap) |
| Manifest gathering (versions, tags, CHANGELOG, issues) | `Explore` subagent |
| Manifest confirmation | Main context (user-facing, must be explicit) |
| Version edits to files | `Edit` tool, main context — each edit reviewable |
| Git commit / tag / push | Bash, main context, sequential with idempotence guards |
| `gh release create`, milestone close, issue close | Bash, main context |
| Roll-over decision for deferred findings | Main context — always ask, never default |

## Common mistakes

| Mistake | Correct |
|---------|---------|
| Running without a GO decision | Hard-refuse — release-readiness must have signed off; running on a NO-GO ships bugs |
| Auto-pulling or auto-rebasing on behalf of the user | Surface divergence from `origin/main`; the user settles it, not the skill |
| Skipping the manifest confirmation | A HARD GATE before anything touches the remote is non-negotiable — a wrong tag is awkward to undo |
| Using `sed` in bash for version edits | Use the `Edit` tool so the change is visible in the tool log |
| Re-tagging when the tag exists | Every step checks idempotence first; re-running a partial close is supposed to be safe |
| Force-pushing a retagged release | Don't — a published tag is effectively immutable; if the tag is wrong, make a new one |
| Closing open finding issues | Only the *consolidated audit report* issues close here; individual findings carry forward |
| Auto-defaulting deferred findings into a target milestone | Always ask the user which milestone, or whether to unassign |
| Forgetting the post-release dev bump | Leaving main at the release version lets the next commit accidentally overwrite the tagged version — dev bump is part of the ritual |
| Writing a release note that differs from CHANGELOG | The CHANGELOG is authoritative; the release note is a direct extract, not a rephrasing |
| Claiming the close is done without checking Phase 5 links | `verification-before-completion` applies here too — the release URL is the evidence |
