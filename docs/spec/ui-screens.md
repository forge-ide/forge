# UI screens

Text wireframes for each v0.1 screen. Layout and behavior are binding; visual styling is not.

## Shell layout and navigation

One window per project. Three dockable columns plus a status bar; panels can be collapsed, resized, and rearranged, and the layout is saved per project.

```
+--------------------------------------------------------------------------------+
| [Forge v]  forge  (main)          [ Cmd+K  Search or run a command ]   (o) 2  |
+-----------------+---------------------------------------+----------------------+
| BOARD           |  [Review] [Editor] [Session]           | TRANSCRIPT           |
|  > Add retry... |                                       |                      |
|    working  2m  |   (center panel: one of the three     |  (session events,    |
|  - Fix flaky... |    views, chosen by tab or by         |   streaming)         |
|    review       |    clicking a board card)             |                      |
|                 |                                       |                      |
| INBOX (2)       |                                       |                      |
|  ! exec: pnpm.. |                                       |                      |
|  ! edit: .env   |                                       |                      |
|                 |                                       |                      |
| FILES           |                                       | [ prompt box      ]  |
+-----------------+---------------------------------------+----------------------+
| ses_01J  claude-code  Accept edits  Standard  $0.42  12.1k tok   LSP: rust ok   |
+--------------------------------------------------------------------------------+
```

- **Title bar:** project switcher, branch, command palette (Cmd/Ctrl+K), and an approvals badge showing the pending count.
- **Left column:** Board, Inbox, and Files, stacked and individually collapsible.
- **Center:** Review, Editor, or Session, for the selected session. Selecting a board card switches all three columns to that session.
- **Right column:** live transcript of the selected session, with the prompt box at the bottom.
- **Status bar:** the selected session's ID, agent, mode, sandbox tier, cost, tokens, and language-server health. Mode and tier are clickable menus.
- **Unsandboxed sessions** show a red tier chip in the status bar, on the board card, and in the review header.

## Agent board and approvals inbox

**Board card** (one per session, newest activity first):

```
+-----------------------------------------+
| Add retry to webhook client     [claude]|
| working · Running cargo test   2m 14s  |
| Accept edits · Standard   $0.42  +84 -12|
| [!] 1 approval waiting                  |
+-----------------------------------------+
```

- Line 1: title (editable inline) and agent badge.
- Line 2: state, current step (latest tool-call title or plan entry), elapsed time in this state.
- Line 3: mode, sandbox tier, cost, lines added and removed.
- Line 4 only when something needs the human: pending approvals, ready for review, failed, or paused by budget.
- Card actions on hover or right-click: open, pause, cancel, rewind, close. A **New session** button sits at the top of the board.
- **New session dialog:** title, prompt (multi-line), agent profile, mode, sandbox tier. Defaults come from the project config. Choosing Unsandboxed shows the warning and requires a checkbox.

**Approval item:**

```
+------------------------------------------------------+
| exec  pnpm add left-pad              Add retry to... |
| Reason: not in allow rules (Accept edits mode)       |
| cwd: .   sandbox: Standard                           |
| [Allow] [Allow for session] [Allow for project] [Deny]|
|  Deny with note: [_____________________________]     |
+------------------------------------------------------+
```

- Edit approvals show a small inline diff of the proposed change.
- **Allow for project** writes an allow rule to `.forge/policy.toml` and shows the rule before saving.
- Keyboard: `A` allow, `S` allow for session, `D` deny, `J`/`K` move between items.
- When the window isn't focused, a new approval raises a desktop notification that opens the item.

## Session view

The Session tab in the center column shows the plan and the timeline; the transcript stays in the right column.

```
+---------------------------------------------------------------+
| Add retry to webhook client          working   [Pause] [Stop] |
+---------------------------------------------------------------+
| PLAN                                                          |
|  [x] Read webhook client and tests                            |
|  [>] Add exponential backoff with jitter                      |
|  [ ] Add tests for retry and give-up paths                    |
+---------------------------------------------------------------+
| TIMELINE                                     [filter: all v]  |
|  12:01  prompt     "Add retry to the webhook client..."       |
|  12:01  read       src/webhook.rs                             |
|  12:02  edit       src/webhook.rs  +41 -3      allow (rule)   |
|  12:02  exec       cargo test webhook          allow (rule)   |
|  12:03  * checkpoint 3                          [Rewind here] |
|  12:03  exec       pnpm add left-pad           ask -> denied  |
+---------------------------------------------------------------+
```

- **Transcript (right column):** agent messages rendered as Markdown, tool calls as collapsible cards with the action, the policy decision and its reason, and truncated output. Terminal output renders in an embedded xterm.js block.
- **Timeline:** one row per event that matters (prompts, tool calls, decisions, checkpoints, approvals, errors). Filter by kind. Selecting a row scrolls the transcript to it.
- **Rewind** asks for confirmation, names the checkpoint, and says how many turns and lines will be discarded.
- **Prompt box:** multi-line; Enter sends, Shift+Enter adds a line. `@` mentions files; `/` lists the agent's advertised slash commands. Disabled while Working, with a Queue option that sends when the turn ends.

## Review pane

The Review tab is the home screen for any session in Review. It behaves like a GitHub pull request's Files tab.

```
+------------------------------------------------------------------------+
| Add retry to webhook client   3 files +84 -12   Round 2  [Since last v]|
| [Request changes (2)]  [Ship v]                    ! 1 new diagnostic   |
+-------------------+----------------------------------------------------+
| FILES             | src/webhook.rs                    [x] Viewed  [...]|
| [x] src/webhook.rs|  41    let mut attempt = 0;                         |
| [ ] src/retry.rs  |  42 +  loop {                                      |
| [ ] tests/hook.rs |  43 +    match send(&req).await {                  |
|                   |  44 +      Err(e) if attempt < MAX => {             |
|                   |     +--------------------------------------------+ |
|                   |     | you: MAX should come from config, not a    | |
|                   |     | constant.                                  | |
|                   |     | claude: Moved to RetryConfig::max_attempts | |
|                   |     | [Reply] [Resolve]                          | |
|                   |     +--------------------------------------------+ |
|                   |  45 +        sleep(backoff(attempt)).await;  (?)   |
+-------------------+----------------------------------------------------+
```

- **Header:** title, file count, totals, review round, a toggle between full changeset and **Since last review**, the Request changes button with the count of open comments, Ship (push branch or merge locally), and a diagnostics summary.
- **File list:** viewed checkboxes persist per review round; generated and lock files are collapsed by default.
- **Diff:** split or unified (toggle, remembered). Click or drag across line numbers to comment on a line or range. Comments draft locally until Request changes sends them all.
- **Hunk menu (`...` per hunk):** Revert hunk, Copy, Show why.
- **Why panel `(?)`:** opens beside the diff with the transcript turn and plan entry that produced the hunk.
- **Diagnostics:** gutter markers on lines with new errors or warnings since the base.
- **Threads** show author, body, and the agent's reply; resolved threads collapse; outdated ones show the original code they were written on.
- **Large diffs:** files over 1,500 changed lines start collapsed with a Load diff button.

## Editor and agent files

- **File tree** in the left column shows the main checkout, or the selected session's worktree when a session is selected (a banner says which). Git status colors files; agent files get a small badge.
- **Editor tabs** in the center column. Saving writes through `file.write` with the base hash; on conflict, show a three-way merge using `@codemirror/merge`.
- **Editing a session's worktree** while its agent is Working is blocked with a message offering Pause. Edits made while Paused or in Review are committed on the session branch so the agent sees them.
- **LSP:** completion, hover, go to definition (F12), references (Shift+F12), rename (F2), and diagnostics, through the daemon.
- **Markdown:** split view with live preview (Cmd/Ctrl+Shift+V toggles). Mermaid blocks render; frontmatter shows as a form above the body when a schema is known.
- **Agent files** (`AGENTS.md`, `CLAUDE.md`, `GEMINI.md`, `SKILL.md`, `.mcp.json`, `.claude/settings.json`, `.forge/*.toml`): schema-driven completion and inline lint. A header strip shows token count and which agents load this file, with a link to the effective-context preview.

```
+---------------------------------------------------------------+
| AGENTS.md   ~1.8k tokens   Loaded by: Codex, Claude (via sync)|
| ! Line 14: references docs/arch.md, which does not exist      |
+-------------------------------+-------------------------------+
| # Forge                       | Forge                         |
| ## Build                      | Build                         |
| - `just ci` before PRs        |  - just ci before PRs         |
+-------------------------------+-------------------------------+
```

## States and default keybindings

Every view handles these states explicitly; none may show a blank panel.

| State | Required behavior |
| --- | --- |
| Daemon not reachable | Full-window banner, "Starting Forge daemon...", retrying with backoff; after 10 s, show the error and a Retry button with log location |
| No project open | Welcome view: Open folder, recent projects |
| No sessions | Board shows a short explanation and the New session button |
| Agent not installed or signed in | Session dialog shows status per profile with Install or Sign in actions |
| Loading | Skeletons sized like the content; no spinners longer than 300 ms without text |
| Session Failed | Red card; session view shows the error, the last 20 log lines, and Retry and Close actions |
| Paused by budget or circuit breaker | Amber card naming the cause, with Resume and Change mode actions |
| Disconnected mid-session | Thin banner "Reconnecting..."; content stays visible and marked stale |

**Default keybindings** (Cmd on macOS, Ctrl on Linux)

| Keys | Action |
| --- | --- |
| Mod+K | Command palette |
| Mod+N | New session |
| Mod+1 / 2 / 3 | Review / Editor / Session tab |
| Mod+Shift+A | Focus approvals inbox |
| Mod+Enter | Send prompt |
| Mod+. | Cancel the current turn |
| Mod+Shift+R | Request changes |
| C | Comment on selected lines (review pane) |
| ] / [ | Next / previous file (review pane) |
| V | Toggle Viewed on the current file |
| Mod+P | Go to file |
| F12, Shift+F12, F2 | Definition, references, rename |

Every binding is a named command, so the palette lists it and users can remap it in `~/.config/forge/keymap.toml`.
