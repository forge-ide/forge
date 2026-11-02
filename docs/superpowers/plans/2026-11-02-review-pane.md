# Review Pane Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans inline. No commits until the user authorizes them.

**Goal:** Inspect, discuss and ship a session's reviewed checkpoint in the desktop app.
**Architecture:** Solid stores own authoritative RPC snapshots and event refreshes. A selected session opens a full-width pane with a read-only CodeMirror diff and a focused transcript panel.
**Tech Stack:** SolidJS, CodeMirror state/view, TanStack Solid Virtual, Vitest with jsdom.
**Spec:** ../specs/2026-11-02-review-pane-design.md (approved in conversation).

## Global Constraints

No forge-api/schema/migration changes. Preserve quoted paths. Render untrusted content as text with visible controls. Ship the reviewed head and require explicit withheld acknowledgement. No PR/commit/push. Second PR waits for the first to be merged. Use fake-agent session tests and GIT_CONFIG_GLOBAL=/dev/null. Run just gen, just ci, just test-sandbox, then security review and Astra dispositions.

## Review Focus

- Late RPCs after session/scope switches cannot replace current content.
- Since-last-review cannot hide unresolved threads or alter revert scope.
- Ship confirmation cannot silently adopt a newer head or acknowledgement.
- Untrusted controls, paths, markup and output cannot disguise code or execute.
- Missing transcript boundaries and empty pages must remain visibly incomplete.

### Task 1: Store and safe display foundations

Files: stores/review.ts, stores/sessions.ts, features/review/text.ts and matching tests.
- [x] Add failing tests for event debounce, reconnect/disposal/races, exact path/scope parameters and reviewed-head/withheld shipping.
- [x] Observe failures with just test-ui; implement authoritative stores and visible text.
- [x] Verify tests pass. The approved setViewed exception reloads changeset on acknowledgement.

Interfaces: createReviewStore(conn, sessionId), createSessionsStore(conn, projectId), visibleText(text).

### Task 2: Read-only diff

Files: packages/editor/src/diff.ts, matching DOM tests, UI DiffView.tsx.
- [x] Test line mapping, new-side range selection, safe text, hunk controls and disposal first.
- [x] Implement CodeMirror gutters, decorations, comment widgets, copy/revert/why callbacks; preserve original API data outside display transformation.
- [x] Run tests and typecheck.

Interfaces: mountDiff(parent, hunks, options) -> destroy(), selection callbacks use inclusive head line ranges.

### Task 3: Review controls and navigation

Files: features/review/{ReviewPane,Threads,ShipDialog}.tsx, features/projects/SessionList.tsx, ui/VirtualList.tsx, App.tsx, styles.css.
- [x] Add rendered tests for warnings, collapse/load, threads/composers, safe rendering, keyboard behavior and ship acknowledgement/head.
- [x] Implement controls using store calls and virtual lists; no component daemon-state mutation.
- [x] Verify loading/empty/error/disconnected states and test pass.

Interfaces: ReviewPane(connection, sessionId, onClose), SessionList(connection, projectId, onSelect).

### Task 4: Focused transcript

Files: stores/transcript.ts, features/review/WhyPanel.tsx and matching tests.
- [x] Test reverse pagination, chosen-turn boundaries, chunk coalescing, partial history, switching/disposal and safe rendering first.
- [x] Implement lazy events.query loading and read-only virtualized turn display.
- [x] Verify tests and integrate Show why.

Interfaces: createTranscriptStore(conn, sessionId, turnId), WhyPanel(connection, sessionId, turnId, onClose).

### Task 5: Finish and review

- [x] Mutation-check HTML safety, reviewed head and withheld acknowledgement; record failures and restore.
- [x] Update binding specs and tick only Review pane UI; document dependencies and follow-ups.
- [x] Run just gen, GIT_CONFIG_GLOBAL=/dev/null just ci, then just test-sandbox; inspect UI.
- [x] Independent security review; consult same GPT-6 Astra agent for findings, fix/re-review.
- [x] Report actual platform coverage and leave changes uncommitted for user review.
