# Smoke checklist

Run before every release, and to close M5. Real agents only; automated tests use `forge-fake-agent`.

For each agent (Claude Code, Codex, Gemini CLI) × each sample repo in `tests/fixtures/repos/` (Rust, TypeScript, Python, Go), on both Linux and macOS:

## Setup

- [ ] Agent shows as installed and signed in on the New session dialog.
- [ ] Sandbox tier is Standard; mode is Accept edits.

## Task

Prompt: "Add a function and a unit test that makes the failing test pass."

- [ ] Session moves Provisioning → Ready → Working without errors.
- [ ] Edits land only in the session's worktree; the main checkout is untouched.
- [ ] Running the test command triggers an approval (not in the allow rules), and approving it works.
- [ ] A command outside the allowlist (for example, `curl` to a non-allowlisted host) is blocked by the sandbox and shown in the timeline.
- [ ] Session reaches Review; the changeset shows every changed file, including new ones.

## Review loop

- [ ] Leave one line comment and click Request changes; the agent replies in the thread and the fix appears under "Since last review".
- [ ] Revert one hunk; the agent sees the revert on its next turn.
- [ ] Rewind to an earlier checkpoint; the worktree and transcript match that point.

## Ship

- [ ] Ship with `merge-local`; the base branch has the change and the session is Done.
- [ ] Ship with `push-branch` to a scratch remote; the branch exists remotely.

## Record

Note agent version, OS, and any failure with its session ID and `events.jsonl` path in the release checklist.
