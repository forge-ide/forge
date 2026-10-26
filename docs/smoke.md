# Smoke checklist

Run before every release, and to close M5. Real agents only; automated tests use `forge-fake-agent`.

For each agent (Claude Code, Codex, Gemini CLI, and on Linux Antigravity) × each sample repo in `tests/fixtures/repos/` (Rust, TypeScript, Python, Go), on both Linux and macOS:

## Before you start

- [ ] Each agent's adapter is installed at the version `forge agent list` names as `supportedVersion`; for one that isn't, `installCommand` is the command to run (`npm install -g ...`).
- [ ] Each agent is signed in: `forge agent list` shows it `authenticated`, or under `signIn` names the command that stores its API key (`secret-tool store --label='Forge NAME' service forge account NAME` on Linux, `security add-generic-password -s forge -a NAME -w` on macOS) or, for Claude Code and Codex, the command that signs in with a subscription (`claude`, `codex login`). Run the checklist once with keys and once with subscription sign-ins; Google sign-ins aren't carried, so Gemini needs a key.
- [ ] Antigravity: install it with the command `forge agent list` gives, then `forge agent sign-in antigravity --method oauth-personal`, which opens your browser; afterwards `forge agent list` shows it `authenticated`, and `~/.gemini/antigravity-acp/acp_token.json` exists. Run it in the Standard tier; Light refuses it.
- [ ] Toolchains are visible in the sandbox. It shows the system directories, `~/.cargo`, `~/.rustup`, and `~/.npm`; one installed elsewhere under the home dir (Go from a version manager, a Python virtualenv) needs its directory in `[sandbox] read_only` in `~/.config/forge/config.toml`.
- [ ] A sample repo outside Forge's own checkout: `just smoke-repo rust /tmp/smoke-rust` (or `typescript`, `python`, `go`), then `forge project open /tmp/smoke-rust`. The repo's README names its test command.

Until the session UI arrives (M8), drive sessions with the CLI: `forge session create`, `forge session watch <id>` in one terminal, and `forge approval list` / `forge approval decide <id> allow|deny` in another.

## Setup

- [ ] Agent shows as installed and signed in: `forge agent list <project id>` (on the New session dialog once it exists).
- [ ] Sandbox tier is Standard; mode is Accept edits: `forge session create --project <id> --agent <claude|codex|gemini> --title "feat: slugify" --sandbox standard --mode accept-edits --prompt "..."`.

## Task

Prompt: "Add a function and a unit test that makes the failing test pass."

- [ ] Session moves Provisioning → Ready → Working without errors.
- [ ] Edits land only in the session's worktree; the main checkout is untouched.
- [ ] Running the test command triggers an approval (not in the allow rules), and approving it works.
- [ ] A command outside the allowlist (for example, `curl` to a non-allowlisted host) is blocked by the sandbox and shown in the timeline.
- [ ] The agent commits with `git_commit`: an approval for the `git` `commit` action appears, and once approved, `git log <branch>` in the repo shows the agent's commit and `git status` in the worktree is clean. A `git commit` the agent runs itself is refused with a pointer to `git_commit`.
- [ ] Session reaches Review; the changeset shows every changed file, including new ones (with the review store; until then, `git diff main...<branch>`).

Agent notes: Codex runs its commands only by escalating them, which asks Forge; a command it runs without escalating fails at its own sandbox. Note how often that happens, and check that every Codex command approval offers a plain "allow once" option (`forge session watch` shows the request): one that offers only "for this session" would let later runs of that command skip Forge.

- [ ] Gemini CLI with a repo `.gemini/policies/` rule allowing `run_shell_command`, or a `.gemini/settings.json` allowing it: commands still ask.
- [ ] Gemini CLI with an installed extension that defines an MCP server named `forge`: the agent's `git_commit` still reaches Forge (an approval for the `git` `commit` action appears), never the extension's server.
- [ ] Each agent picks up the user's settings (its model, say) and instructions (`~/.claude/CLAUDE.md`, `~/.codex/AGENTS.md`), and a committed `.claude/settings.json` rule.
- [ ] A command that writes `.claude/settings.local.json` granting `Bash` doesn't stop later commands from asking; Codex or Gemini restarted after a command rewrites `.codex/config.toml` or `.gemini/settings.json` refuses to start, naming the file.
- [ ] With a subscription sign-in, `~/.claude/.credentials.json` and `~/.codex/auth.json` are unchanged after the session, and the agent's own CLI still works outside Forge.

## Review loop

From the review milestone on:

- [ ] Leave one line comment and click Request changes; the agent replies in the thread and the fix appears under "Since last review".
- [ ] Revert one hunk; the agent sees the revert on its next turn.
- [ ] Rewind to an earlier checkpoint; the worktree and transcript match that point.

## Ship

From the review milestone on:

- [ ] Ship with `merge-local`; the base branch has the change and the session is Done.
- [ ] Ship with `push-branch` to a scratch remote; the branch exists remotely.

## Record

Note agent version (`forge agent list`), OS, and any failure with its session ID and `events.jsonl` path in the release checklist.
