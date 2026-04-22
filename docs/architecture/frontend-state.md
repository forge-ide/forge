# Frontend state: session identity is signal-sourced

## Decision

The active Forge session's identity (`sessionId`) and its workspace root
(`workspaceRoot`) live in two global Solid signals in
`web/packages/app/src/stores/session.ts`:

- `activeSessionId: Signal<SessionId | null>`
- `activeWorkspaceRoot: Signal<string | null>`

Every webview component that depends on the active session reads these
signals directly. Components **do not** accept `sessionId` as a prop.

The signals are written in exactly one place: `SessionWindow.onMount`, which
sets them from its route params and from the `HelloAck.workspace` reply.
`SessionWindow.onCleanup` clears them. No other module writes.

## Status

Accepted — F-385.

## Rationale

Before this decision the codebase carried a mixed pattern:

- `StatusBar`, `FilesSidebar`, `EditorPane`, and `ChatPane`'s
  `BranchedAssistantTurn` sub-component required `sessionId: SessionId` as
  a prop, passed down from `SessionWindow`.
- `ChatPane` itself, `ToolCallCard`, the `ChatPane` composer, and the chat
  send path read `activeSessionId()` directly from the signal.

When the prop source and the signal drifted — e.g. during a tab-swap where
`SessionWindow` updates the signal on the *new* id while a ticking
microtask still holds a closure over the prop value of the *old* id — the
two paths could disagree for a frame. The frame of drift is small, but the
same pattern is also unsafe for the roadmap: multi-session-per-window (a
window hosting more than one session) and background-render of an inactive
session both require each pane to name the session it is bound to without
assuming it equals "the signal's current value." The mixed pattern locks
those features out because half the tree can't tell one session from
another.

Routing everything through the signal was chosen over threading a uniform
prop (or adding a `createContext`) because:

1. **One writer, one reader.** `SessionWindow` is the only writer of the
   signal. Every consumer reads the same signal. There is nowhere for
   drift to occur.
2. **Net code reduction.** Every consumer shed a prop and a matching type
   field. `SessionWindow` dropped the forwarding sites.
3. **Reactivity is correct by construction.** Reads inside event handlers
   and effects see the current value when they run, not a mount-time
   snapshot.

A `createContext` would have solved the drift too, but it adds a layer of
indirection that no other pane needs today; the signal is already the
single source, so the context would just wrap it.

## Future work when multi-session-per-window lands

When a single webview needs to render panes bound to *different* sessions
at once — not the Phase 2 shape, but a roadmap item — the signal pattern
will have to change. At that point introduce a `SessionScope` provider
(a `createContext<SessionId>`) that a component instance can be bound to;
panes read the scope's `sessionId` instead of `activeSessionId()`. The
global signal stays as "the foreground session" for UI chrome that is
inherently single-valued (window title, activity bar, etc.). Every pane
that moves onto the scope must drop its implicit dependence on
`activeSessionId()` — and the DoD invariant from F-385 stays in force:
no pane both reads the scope and accepts `sessionId` as a prop.

## How to enforce

- Do not add a `sessionId` prop to pane/shell components. If a pane needs
  it, read `activeSessionId()` inside the function body or the effect
  that consumes it.
- Do not write to `setActiveSessionId` outside `SessionWindow`.
- The per-component F-385 tests (`StatusBar.test.tsx`,
  `FilesSidebar.test.tsx`, `EditorPane.test.tsx`) pin the invariant by
  mounting without the prop and asserting the component reads the signal
  value. Keep them green.
