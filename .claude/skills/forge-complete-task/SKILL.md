---
name: forge-complete-task
description: Use when picking up and completing a Forge task end-to-end — chains task claiming, TDD implementation, and PR submission
---

# forge-complete-task

## Overview

End-to-end Forge task workflow. Three phases in strict order: **claim → implement → submit**. The main context is a coordination layer — delegate every discrete unit of work to subagents. Do not explore, search, or implement inline.

## Phase 1: Claim

Invoke `forge-start-task`. Complete all steps before writing any code:
- Sync main with upstream
- Pick an unclaimed issue
- Create feature branch
- Read and record the full Definition of Done

**Gate:** Do not begin Phase 2 until you have the DoD checkboxes in hand.

## Phase 2: Implement

**Delegate all exploration to an `Explore` subagent before writing any code.** The subagent should map the relevant crates, existing patterns, test conventions, and public APIs needed to implement the DoD. Bring only the findings summary back to the main context.

If the DoD involves a UI view, pane, component, or interaction pattern, invoke `frontend-design:frontend-design` before writing any code. Use it to establish purpose, aesthetic direction, and key interaction states. Skip this if the task is purely backend.

Invoke `superpowers:test-driven-development` for every change. No exceptions.

- RED: write a failing test for each DoD behavior (delegate test runs to Bash, keep results terse)
- GREEN: minimal code to pass
- REFACTOR: clean up, stay green

**Gate:** All tests pass and the DoD is fully addressed before proceeding.

## Phase 3: Submit

Invoke `forge-finish-task`. That skill already spawns fresh-context subagents for DoD verification and code review — let it do so. Do not inline those checks.

Steps delegated inside `forge-finish-task`:
- Fresh-context general-purpose subagent verifies every DoD checkbox independently
- Build verification (`cargo fmt --check && cargo check && cargo test && cargo clippy`)
- Fresh-context `feature-dev:code-reviewer` subagent reviews changed files
- PR opened to `forge-ide/forge`
- Issue label updated to `status: code-review`

**If forge-finish-task finds gaps:** fix them (return to Phase 2), then re-run Phase 3. Do not re-run Phase 1.

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
