# Review pane design

## Intent and decisions

Build the review pane on the completed daemon review API, so a human can inspect
exactly what a session changed, discuss it with the agent, and ship the reviewed
checkpoint. The backend is the authority for permitted actions and refusals.

The user approved these decisions in this conversation:

- Comments are daemon threads used as persistent drafts. Creating a comment or
  reply does not send an agent prompt; Request changes sends eligible threads.
- A per-project session list under ProjectList opens a full-width review pane
  through `selectedSession` in `App.tsx`. No router dependency.
- Show why and the `(?)` control must show the originating transcript turn now.
  The Why panel's plan entry remains outside this change.
- GPT-6 Astra replaces Fable for security-finding dispositions, with the same
  agent resumed across rounds.

The original request also approves unified diff first (split only if time
allows) and explicitly excludes the diagnostics gutter and header summary,
Why plan entry, GitHub sync, board/session view and timeline rewind. Update
the binding UI spec to record those scoped exceptions in this change.

Approved project memory remains authoritative for checkpoint, thread, revert,
ship, harness, and performance decisions. This document does not reopen them.

## Approaches

For comments, local drafts would need separate persistence and reconciliation.
The user selected daemon threads as persistent drafts, using the existing API.

For navigation, the eventual board and center tabs require unrelated session UI.
The user selected the existing ProjectList entry point and a full-width pane.

For Show why, deferring it would leave hunk provenance inaccessible. The user
requested the transcript now. A focused, read-only turn panel uses `events.query`
without introducing the general Session view, timeline, editor, or prompt box.

## Screen and design contract

This is an Operate surface: inspecting changes and acting on a review. Extend
Forge's current system font, warm neutral surfaces, rust accent, semantic colors,
thin borders, and light/dark tokens. Do not introduce decorative cards, marketing
copy, new fonts, imagery, or a second navigation shell.

The full-width pane has a compact header, file navigation on the left, and one
selected file's unified diff. Selecting another file replaces the diff rather
than mounting all file contents. The Why panel opens beside the diff. On narrow
windows, file navigation and Why can stack while the code retains horizontal
scrolling. Controls have labels, visible focus, and keyboard access.

Header: session title, file count, additions/deletions, round, full/Since last
review toggle, Request changes with unresolved comment count, and Ship. An
unsandboxed session carries a visible warning. Unknown counts from `tooLarge`
files are explicitly identified; totals never imply those files have no changes.

File navigation: repo and path identify a file; old path and modes are visible
where relevant. Viewed checkboxes use `review.setViewed`. Generated and lock
files, binary files, and files over 1,500 changed lines initially show a collapsed
diff with Load diff. Collapsing is a display choice, never a claim of review.
`tooLarge` and refused diffs instead show warnings marked not reviewed, with
viewed unavailable. Preserve the original quoted path in all API parameters.

Diff: a read-only CodeMirror view, with old/new line numbers. Clicking or dragging new-side
line numbers selects an inclusive range. Only new-side lines can anchor a new
comment: the API anchors at head. Show the selection, a textarea, save and cancel;
prevent a selection from leaking across files or changeset refreshes. `C` opens
the composer, `]` and `[` move files, `V` toggles viewed, and Mod+Shift+R requests
changes. Shortcuts do not fire in text entry controls or during IME composition.

Threads: author and body for each human/agent comment; Reply and Resolve;
resolved threads collapse but can be expanded. Outdated threads show their quote.
Threads for files absent from Since last review remain reachable in a session
threads section, so unresolved comments cannot disappear behind the diff filter.
All user input remains in the composer when its daemon call fails.

Hunk controls: Copy copies the displayed unified hunk as text. Revert hunk passes
the same repo, exact path, and `fromCheckpoint` as that hunk's diff. Show why and
`(?)` open the turn panel when `turnId` exists. When it is absent, explain that
origin attribution is unavailable rather than inventing a turn.

## Transcript panel

Use only existing `events.query`, with `sessionId`, `limit` at most 1,000, and
exclusive `before` pagination. Read backwards in bounded pages to locate the
selected `turn.started`/`turn.ended` boundaries. Display progress while locating
older turns; allow closing or switching to cancel obsolete work. Do not download
transcript history until Show why is invoked.

Between the chosen turn's boundaries, render prompt text, agent message chunks
coalesced by message ID, tool call titles/actions, policy decisions, redacted tool
results, and terminal output as text. Tool/output details are collapsible. Use
event sequence order and the parent/tool-call identifiers for association. Do not
attribute a result outside those turn boundaries to the selected turn. Plan
entries are excluded. A missing/pruned turn gets a specific empty state. Partial
history is labeled as such, not presented as a complete turn. Errors are retryable.

The panel is read-only, not the future general transcript screen. No Markdown,
HTML, terminal escape interpretation, active links, or executable content.

## State ownership and refresh

Add session, review, and focused transcript stores under `packages/ui/src/stores/`.
Components hold only interaction state such as selection, composers, expanded
sections, and acknowledgement. Stores own authoritative daemon data and calls.

Load on connection and reload on reconnection or subscription reset. Subscribe to
the appropriate project/session topics. Session state, checkpoints, rewinds,
turns and `review.thread` invalidate affected reads. Debounce bursts of thread
events with a bounded delay; clean up timers/listeners/subscriptions on disposal.
Generation checks prevent late responses from replacing newer selections or
data. Do not refetch all diffs for each thread event.

Each data view handles loading, empty, error with retry, and disconnected/stale.
Retain stale content visibly on disconnect and disable mutations. Daemon errors
include their useful structured details (especially Conflict file lists) as safe
text. A failed refresh must not silently replace the prior content with empty
arrays. A successful mutation never overwrites data from its returned object in
a component; event-driven refreshes obtain the authoritative view.

### Approved exception: viewed acknowledgement without an event

`review.setViewed` persists the mark but emits no daemon event. This conflicts
with the literal event-only update rule in AGENTS.md. Proposed narrow exception:
the review store reloads `review.changeset` after a successful `setViewed`, with
no optimistic mark; it also reloads on reconnect and review invalidation events.
No new wire event or interface change. The user approved this exception in the conversation before implementation. Other mutations with events retain event-driven refreshes.

## Ship and checkpoint consistency

Capture the changeset's reviewed `head` when opening Ship. Both push branch and
merge locally send it as `checkpointId`; never substitute a newly fetched head
behind an open confirmation. Close/reset the dialog or require fresh confirmation
when the review scope changes. Render Conflict details and leave the daemon in
charge of freshness, identity, branch validity, and permitted state.

List locally known withheld files before Ship, and show any files named by a
daemon refusal. Initially omit `includeWithheld`. Only an explicit unchecked
acknowledgement for the currently reviewed checkpoint enables a request with
`includeWithheld: true`. Reset that acknowledgement when head/session changes or
the dialog closes. A push has an optional remote branch field; merge has none.

## Untrusted text and scale

Use Solid text nodes exclusively for diffs, names, bodies, replies, quotes,
transcript and errors. Never use `innerHTML`. A shared display function makes
control characters, bidi controls and Unicode Default_Ignorable_Code_Point characters visible
using unambiguous code-point labels; retain structural line breaks separately
and preserve U+FFFD as received. Transformation is for display only: original
quoted paths are retained as API identifiers. Copy is plain text and cannot
execute content.

Honor component-architecture.md's CodeMirror renderer requirement: put the
read-only diff extension in `packages/editor`, with line-number gutters,
decorations and comment widgets, creating the view only for the selected,
loaded file and destroying it on deselection. CodeMirror virtualizes the code
viewport. Use `@tanstack/solid-virtual` for file/session lists, transcript rows
and thread/comment lists that can exceed 100 entries. Avoid eagerly loading
collapsed or unselected diffs. Code preserves whitespace and horizontal
scrolling; virtualized prose rows use measurement. A unified-only renderer is
the scoped first version approved in the request; update ui-screens.md to say
so rather than claiming a remembered split/unified toggle exists.

Production dependencies: `@tanstack/solid-virtual`, `@codemirror/state`, and
`@codemirror/view` (the latter two provide the specified diff renderer in the
currently stubbed `packages/editor`). Add a DOM test environment only
if needed for meaningful component tests; prefer direct Solid render and DOM
assertions over adding a separate testing-library dependency. Verify dependency
licenses, maintenance and exact versions before installation and document them
in the eventual PR description.

## Validation and delivery

Write failing store and rendered component tests before implementation. Pin the
selection range, quoted paths, same diff/revert scope, thread persistence and
debouncing, resolved/outdated behavior, reconnect/disposal races, safe rendering,
reviewed-head shipping, withheld acknowledgement reset, Conflict lists, turn
pagination, and unavailable attribution. Use the fake agent for session flows.

Mutation-check safe text rendering, reviewed `head`, and withheld acknowledgement
by deliberately breaking each behavior and observing a relevant test fail, then
restore and rerun. Keep evidence of commands and outcomes.

Run `just gen` without editing generated files; no forge-api, config schema, or
SQL migration changes are expected. Run `just ci`, including a run with
`GIT_CONFIG_GLOBAL=/dev/null`, then `just test-sandbox`. Obtain an independent
security review of the diff. Consult the same GPT-6 Astra agent for each security
finding; fix ordinary correctness issues directly and re-review after fixes.
Scope or security-default changes go to the user.

Inspect the rendered UI for clipping, focus, stale/error states and inert
controls. Tick Review pane UI in build-plan only after it is implemented and
verified. Keep the milestone open: the fake-agent end-to-end flow and exact
5,000-line first-file < 400 ms (median of five), maximum rAF delta <= 32 ms checks
belong to the second PR after the user merges the first. Linux real-app and macOS
CI proxy results must be reported separately and honestly.

Do not commit, push or open a PR without the user's instruction. Leave moving
session worktrees, squash shipping, Git LFS notes and accept hunk as follow-ups.
