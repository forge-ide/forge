---
name: forge-complete-task
description: Use when picking up and completing a Forge task end-to-end — chains task claiming, TDD implementation, and PR submission. Accepts an optional F-### argument to target a specific task instead of picking one from the backlog.
---

# forge-complete-task

## Overview

End-to-end Forge task workflow. Three phases in strict order: **claim → implement → submit**. The main context is a coordination layer — delegate every discrete unit of work to subagents. Do not explore, search, or implement inline.

## Arguments

| Argument | Form | Meaning |
|----------|------|---------|
| Task identifier (optional) | `F-###` (e.g. `F-035`) | Skip backlog selection and claim this specific task |

If no argument is provided, fall back to the normal backlog-picking flow in `forge-start-task`.

## Phase 1: Claim

Invoke `forge-start-task`. Complete all steps before writing any code:
- Sync main with upstream
- Select the issue (see below)
- Create feature branch
- Read and record the full Definition of Done
- Run pre-work validation (draft plan, resolve info gaps, refine DoD if needed)

**Issue selection:**
- **If an `F-###` argument was provided:** skip the backlog listing/picking steps. Resolve the F-number to its GitHub issue number via `gh issue list --repo forge-ide/forge --state open --search "F-### in:title" --json number,title`. If it is already labelled `status: in-progress` or `status: code-review`, stop and confirm with the user before continuing. Then run the claim, branch, and read steps from `forge-start-task` against that issue.
- **If no argument was provided:** run `forge-start-task` as written — list, pick an unclaimed issue, claim, branch, read.

**Phase 1 outputs (carry forward to Phase 2):**
- Final DoD checkboxes (post-refinement, if any)
- Implementation plan scaffold (files, test strategy, key decisions)
- Resolved answers to any clarifying questions asked

**Gate:** Do not begin Phase 2 until the DoD is finalized **and** the plan scaffold + resolved gaps are recorded.

## Phase 2: Implement

**Delegate all exploration to an `Explore` subagent before writing any code.** The subagent should map the relevant crates, existing patterns, test conventions, and public APIs needed to implement the DoD. Bring only the findings summary back to the main context.

If the DoD involves a UI view, pane, component, or interaction pattern, invoke `frontend-design:frontend-design` before writing any code. Use it to establish purpose, aesthetic direction, and key interaction states. Skip this if the task is purely backend.

Invoke `superpowers:test-driven-development` for every change. No exceptions.

**Hand TDD the Phase 1 context explicitly:** the finalized DoD, the implementation plan scaffold, and the resolved answers to any clarifying questions. TDD should treat the plan as the starting shape (files, test strategy, key decisions) — not re-derive it — and drive each RED test from a DoD checkbox.

- RED: write a failing test for each DoD behavior (delegate test runs to Bash, keep results terse)
- GREEN: minimal code to pass
- REFACTOR: clean up, stay green

**Gate:** All tests pass and the DoD is fully addressed before proceeding.

## Phase 3: Submit + hand off for gates

Invoke `forge-finish-task`. That skill verifies the build, pushes the branch, opens the PR, sets `status: code-review`, and emits a **`HANDOFF-REQUEST`** — but it does **not** run DoD verification or code review. Those gates run in the parent context so the verifier's context stays independent of the implementer's.

Steps delegated inside `forge-finish-task`:
- Build verification (`cargo fmt --check && cargo check && cargo test && cargo clippy`)
- Branch push to `origin` (fork)
- PR opened to `forge-ide/forge`
- Issue label updated to `status: code-review`
- HANDOFF-REQUEST returned to parent (PR URL, issue number, branch, changed files, DoD, design note)

### Parent's responsibility after receiving HANDOFF-REQUEST

The parent (the surrounding context that invoked `forge-complete-task`) must run both gates before merging. **A PR without completed gates must not be merged** — the whole point of the handoff is that the gates actually run.

1. **Dispatch a fresh-context DoD-verifier agent** (general-purpose subagent). Prompt template:
   > Verify the following Definition of Done checkboxes for the forge repo at `/home/jeroche/repos/forge`. For each item, find concrete code evidence (grep result, file read, or test run). Do NOT accept "it compiles" as evidence — find the actual symbol, method, or file.
   >
   > Definition of Done:
   > `<paste DoD from HANDOFF-REQUEST>`
   >
   > Changed files:
   > `<paste changed_files from HANDOFF-REQUEST>`
   >
   > For each checkbox report `VERIFIED: <what you found and where>` or `NOT FOUND: <what is missing>`. Do not guess.

2. **Dispatch a fresh-context code-reviewer agent** (`pr-review-toolkit:code-reviewer` or equivalent). Prompt template:
   > Review PR `<pr_url from HANDOFF-REQUEST>` for forge-ide/forge issue #`<issue_number>`.
   >
   > Definition of Done:
   > `<paste DoD>`
   >
   > Verify: (1) every DoD item is behaviorally correct, not just structurally present; (2) no regressions in existing tests or public APIs; (3) Rust-specific issues — missing derives, incorrect serde attributes, clippy violations; (4) report only HIGH-confidence findings, skip style nitpicks.

3. **If either gate surfaces issues:** re-engage the orchestrator (new Agent invocation with the findings embedded). The orchestrator returns to Phase 2, fixes, amends the branch, re-pushes, and re-emits HANDOFF-REQUEST. Parent re-runs the gates. Loop until both clean.

4. **If both gates are clean:** merge the PR. Only then is the task complete.

### Failure modes to watch for

- `forge-finish-task` exits without a HANDOFF-REQUEST — the orchestrator aborted early. Do not merge blindly; investigate.
- A gate fails but the orchestrator "fixed" it inline without re-emitting HANDOFF-REQUEST — that means the fix was never re-verified. Re-run the gates against the amended branch.
- Merging on a timer / auto-merge without waiting for gates — the gates exist precisely to prevent this. Merge manually after gates confirm clean.

## Delegation Rules

| Work type | Where it runs |
|-----------|--------------|
| Codebase exploration, file reading, grep | `Explore` subagent |
| DoD verification | fresh-context general-purpose subagent (via `forge-finish-task`) |
| Code review | fresh-context `feature-dev:code-reviewer` subagent (via `forge-finish-task`) |
| Build/test commands | Bash (results summarized, not dumped) |
| Targeted file edits based on findings | Main context only |

## Common Mistakes

| Mistake | Correct |
|---------|---------|
| Writing code before reading the DoD | Always finish Phase 1 first |
| Exploring the codebase inline | Spawn an `Explore` subagent |
| Skipping TDD "just for simple changes" | TDD applies to every change |
| Skipping frontend-design for UI tasks | Invoke it before any UI code — design direction first |
| Pushing before build/review pass | `forge-finish-task` gates the push |
| Re-claiming a new issue after gaps found | Loop Phase 2 → Phase 3 only |
| Inlining DoD or code-review checks | Let `forge-finish-task` spawn those subagents |
| Claiming an `F-###` already in-progress without asking | Confirm with the user before taking over |
