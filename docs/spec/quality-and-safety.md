# Quality and safety

What "done" means beyond features: tests, performance, security, failure behavior, and release.

## Test strategy

Most of Forge is tested without a real model. The fake agent makes sessions deterministic; real agents run only in smoke tests.

**Fake ACP agent** (`crates/forge-fake-agent`): a real ACP agent binary that replays a YAML script.

```yaml
# tests/scripts/edit_and_test.yaml
steps:
  - expect_prompt: "*retry*"
  - plan: ["Read client", "Add backoff", "Add tests"]
  - read: src/webhook.rs
  - write: { path: src/webhook.rs, patch: fixtures/backoff.diff }
  - exec: ["cargo", "test", "webhook"]
  - request_permission: { action: exec, argv: ["pnpm", "add", "left-pad"] }
  - expect_permission: deny
  - message: "Done. Added backoff with jitter."
  - end_turn
  - expect_review_comments: 1        # waits for Request changes
  - write: { path: src/webhook.rs, patch: fixtures/config_max.diff }
  - reply_thread: { index: 0, body: "Moved to RetryConfig::max_attempts" }
  - end_turn
```

Each step is a one-key map or a bare name; unknown steps or keys are errors. Paths are relative to the session's working directory. Implemented so far: `expect_prompt` (a glob where `*` matches anything), `plan`, `read`, `write` with `content` (the whole new file), `exec`, `request_permission` (`action: exec` with `argv`, `action: edit` with `path`, `action: read` with an optional `path`, or `action: fetch`), `mcp_call` (a tool on the client's `forge` MCP server, with `arguments` and an optional `expect: deny`), `expect_permission`, `message`, `end_turn`, and `sleep_ms` (waits, unless the turn is cancelled first; for tests that act mid-turn). `write` with `patch`, `expect_review_comments`, and `reply_thread` arrive with the review store. A cancelled turn resumes the script after that turn's `end_turn`. A failed expectation fails the turn with an error.

| Layer | Tooling | Required coverage |
| --- | --- | --- |
| Unit | `cargo test`, vitest | Policy matching, config parsing, diff mapping, thread anchoring, Solid stores |
| Golden | `insta` snapshots | Policy decisions: a table of ~200 actions × modes × rule sets with expected verdicts |
| Differential | Rust test running `bash` with recording stub programs | Policy command parsing: every program bash runs for a corpus of scripts is seen by policy, or the line is marked opaque; a line policy calls simple runs exactly its argv |
| Integration | Rust tests against a real `forged` in a temp dir | Every API method; every session state transition; reconnect with `afterSeq` |
| Sandbox escape | A probe binary run inside each tier, on Linux and macOS CI (`just test-sandbox`); a control run without a sandbox checks every probe can detect its escape, and a tier the machine can't run fails the suite | Must fail: read `~/.ssh/id_*` or other files in the real home, write outside the worktree, reach a non-allowlisted host over TCP or UDP, read another session's worktree or write its git dir, signal a daemon process, read or write the main checkout, write the repo's `.git/hooks` or `.git/config`, connect to the daemon socket or the session bus, read a daemon process's environment, read the real keyring (the login keychain file on macOS, the session bus on Linux, where present). Also must fail: a terminal's own environment replacing a variable the sandbox sets (including through a name containing `=`), and, on Linux, a loader variable (`LD_PRELOAD`) running a library from the worktree outside the sandbox; the control run shows the library does run when nothing confines it. Also: the fake agent's session test passes inside each tier, and killing a Standard sandbox ends processes that left its process group. Through the egress proxy, in every tier including Unsandboxed: an allowlisted host is reachable, and hosts off the allowlist, allowlisted names that resolve to private addresses, IP literals, and other ports are refused and logged; and a request on a secret's route reaches only its fixed host, carrying the real value, while the agent sees only a placeholder; route requests without the per-proxy token, from a browser, with another method, or to a non-inference path are refused |
| End-to-end UI | WebdriverIO with `tauri-driver` on Linux; Playwright against the browser build on macOS | New session → approval → review → request changes → ship, using the fake agent |
| Smoke | Manual checklist in `docs/smoke.md` | Each real agent, each sample repo, before every release |

**Sample repos** in `tests/fixtures/repos/`: small Rust, TypeScript, Python, and Go projects, each with a failing test the smoke task fixes.

## Performance targets

Measured on a reference machine: a 4-core laptop with 16 GB RAM, on Linux (WebKitGTK) and an M-series Mac. CI runs the benchmarks and fails on a regression of more than 20%.

| Measure | Target |
| --- | --- |
| `forged` cold start to accepting connections | < 300 ms |
| App launch to interactive board | < 1.5 s |
| `forged` idle memory | < 60 MB |
| Per-session daemon overhead (excluding agent and language servers) | < 25 MB |
| Policy decision without classifier or hooks | p99 < 2 ms |
| Event from agent to rendered in UI | p95 < 50 ms |
| Session provisioning (worktree + Standard sandbox + agent launch) | < 3 s, excluding agent startup |
| Checkpoint on a 50k-file repo | < 500 ms |
| Review pane: open a 5,000-line changeset | First file visible < 400 ms |
| Review pane: scroll that changeset | No frame over 32 ms |
| Transcript with 2,000 events | Scrolls without dropped frames; memory < 150 MB for the webview |

## Threat model

Forge assumes the agent is well-meaning but fallible and steerable by anything it reads. It protects the user's machine, credentials, and other code from that agent. It does not protect against a compromised user account or a malicious Forge build.

**Assets:** SSH keys, cloud and API credentials, the user's other repos and files, git history on remotes, the network the machine sits on, and the integrity of what gets shipped.

| Threat | Example | Primary defense | Backup |
| --- | --- | --- | --- |
| Careless agent | `rm -rf` in the wrong directory; force-push | Sandbox limits writes to the worktree; pushes only from the daemon | Deny rules; checkpoints |
| Prompt injection | A README or web page tells the agent to send `~/.aws` somewhere | Sandbox hides `$HOME`; egress allowlist | Injection flags; classifier (v0.2) |
| Secret leakage | Agent prints an API key into a log or commit | Secrets injected by the proxy, never in the environment | Secret scan on edits, output, and commits; log redaction |
| Malicious repo | Build script or git hook runs code on checkout | All agent commands run sandboxed; git hooks disabled in worktrees | Container tier for untrusted repos |
| Agent config in the repo | A cloned repo's `.claude/settings.json`, `.gemini/`, `.env`, or `.codex/config.toml` pre-allows tools, names hooks or MCP servers, or picks a mode that skips asking | Agents read the repo's committed config as they would outside Forge, by the user's choice. Claude gets it from the branch's last commit in a read-only file, and Codex and Gemini won't start while theirs differs from that commit, so a command can't add rules mid-session or for the next launch. Forge still sets each agent's mode and checks it took, refuses Claude's bypass mode, ranks Gemini's policy above every settings allow, pins Codex's escalation reviewer to the user, protects agents' project config from edits, and recreates the agent's home config each launch | What a repo's committed settings allow runs without asking Forge, inside the sandbox. A committed hook that runs a script from the worktree runs whatever the script says now |
| Agent sign-ins | A login's tokens leak from the sandbox, or the agent refreshes them and logs the user's own CLI out | Tokens stay in `forged`; the agent gets placeholders and the proxy sends the access token only to the login's host and inference paths; the refresh token never leaves the user's file, which Forge never writes | An expired login fails requests until the user runs the agent's CLI to refresh it |
| Credentials in the user's agent settings | An MCP server's token in `~/.claude.json`, or a key in a settings `env`, reaches the agent | As outside Forge, the agent can read them; copies are the user's only, removed at session end, and the values are registered with the log redactor so they never reach events and commands carrying them ask | The agent can still send one to the model, or to an allowlisted host. Credentials in files under the linked dirs (a plugin's `.mcp.json`) aren't redacted |
| Malicious dependency | Typosquatted package added by the agent | New-dependency guard (v0.2); Ask for package installs by default | Review shows lockfile changes |
| Plugin or hook abuse | A project plugin exfiltrates data | Project plugins pinned by hash and approved on first use; WASM hooks have no ambient access | Hooks run in the sandbox by default |
| Another local user | Connects to the daemon socket | Socket mode 0600 in a per-user directory | Remote clients need paired tokens |
| Review evasion | Agent changes files the review doesn't show | Review is the worktree's git diff, including untracked files | Ship refuses if the worktree changed since the last review |

**Trust boundaries:** UI ↔ daemon (socket permissions, tokens) · daemon ↔ sandbox (only ACP stdio, the MCP endpoint, and the proxy cross it) · sandbox ↔ network (proxy) · daemon ↔ git remotes (user credentials, daemon only).

**Unsandboxed tier** removes the first defense for the careless-agent, injection, and malicious-repo rows. The UI says so when it is chosen.

## Failure handling

The rule throughout: never lose work, never resume silently, and always tell the user what happened and what they can do.

| Failure | Required behavior |
| --- | --- |
| Agent process crashes | Checkpoint the worktree, mark the session Paused with the exit code and last stderr lines, offer Resume (reload the ACP session or start fresh with a summary) |
| Agent hangs (no ACP traffic for 5 min while Working) | Send ACP cancel; if no response in 30 s, kill it and treat as a crash |
| Daemon crashes or restarts | On start, Working and Waiting sessions become Paused and Provisioning sessions Failed, with "Interrupted by daemon restart"; pending approvals are cancelled and the agent is told on resume |
| UI disconnects | Daemon keeps running sessions; approvals wait; UI reconnects with `afterSeq` |
| Sandbox fails to start | Session goes Failed with the backend's error and a suggested fix (missing bubblewrap, unprivileged user namespaces disabled); never falls back to a weaker tier silently |
| Worktree creation fails | Failed with the git error; partial worktrees, and branches the attempt created, removed |
| Rewind or checkpoint git error | Abort the operation, leave the worktree untouched, show the error |
| Ship conflicts with the base branch | Show the conflicting files; offer "Ask agent to rebase" (a new turn) or manual resolution in the editor |
| Disk below 1 GB free | Pause all Working sessions and refuse new ones until space is freed |
| Language server crashes | Restart with backoff; after 3 crashes in 5 minutes mark the language degraded; never block sessions |
| Hook times out or errors | Log it; a `tool.pre` or `edit.pre` hook asks, `commit.pre` and `task.done` block, and post hooks have no opinion |
| Network down | Agent turns fail with the agent's error; session goes Paused; Resume works when back online |
| SQLite corruption | Refuse to start, keep the file, point to `forge doctor --rebuild-index`, which rebuilds from JSONL logs and git refs |

`forge doctor` checks sandbox prerequisites, socket permissions, agent installs, language servers, and disk space, and prints fixes.

## Packaging and CI

**CI** (GitHub Actions)

| Job | Runs on | Steps |
| --- | --- | --- |
| `check` | ubuntu-latest, macos-latest | `just ci`: fmt, clippy (warnings as errors), cargo tests, eslint, `tsc --noEmit`, vitest, generated-types drift check |
| `sandbox` | ubuntu-latest (bubblewrap installed, AppArmor's unprivileged user namespace restriction lifted; Light needs a kernel with Landlock ABI 6), macos-latest | `just test-sandbox`: escape suite and sessions in each tier |
| `e2e` | ubuntu-latest with Xvfb; macos-latest (browser build) | UI end-to-end with the fake agent |
| `bench` | Self-hosted reference machine, nightly | Performance targets; fail on > 20% regression |
| `release` | Tag push | Build, sign, package, publish |

**Linux packages**

- AppImage, `.deb` (Ubuntu 22.04+, Debian 12+), and `.rpm` (Fedora 40+, RHEL 9+).
- Arch Linux: a `PKGBUILD` in `packaging/arch/` builds a `.pkg.tar.zst` inside an `archlinux` container in the `release` job, attached to each release. Two AUR packages: a `-bin` package that installs the release artifact, and a `-git` package that builds from source. Dependencies: `webkit2gtk-4.1`, `bubblewrap`. Choose an AUR package name that isn't already taken before M8.
- Runtime dependencies: WebKitGTK 4.1, bubblewrap. `forge doctor` reports what's missing.
- `forged` ships a systemd user unit; the app installs and enables it on first run with the user's consent.

**macOS packages**

- Universal is not needed; Apple silicon only. A signed and notarized `.dmg` with hardened runtime.
- `forged` is embedded in the app bundle and registered as a launchd agent via `SMAppService`.
- The daemon needs no special entitlements; the Container tier requires the user to install Podman.

**Versioning and updates**

- One version for app, daemon, and CLI (semver). The app refuses to talk to a daemon of a different major API version and offers to restart it.
- Auto-update through the Tauri updater with signed manifests (v0.2); v0.1 checks for updates and links to the release page.
