// Workspace Chat pane — lists sessions belonging to the active workspace
// and lets the user jump between them. Conceptually the "sessions index"
// for the workspace window.
//
// Two filters apply to the full `session_list` payload:
//   1. Rows whose `workspaceRoot` matches `props.workspaceRoot` — keeps
//      this window scoped to its workspace.
//   2. When `props.workspaceRoot` is null we fall back to showing nothing;
//      the active route still drives which session is highlighted.
//
// Activating a session no longer crosses the IPC boundary — we navigate
// the router in-place. The window label is `workspace-<id>` and stays
// the same; only the URL changes.

import { type Component, For, Show, createMemo, createResource } from 'solid-js';
import { useNavigate, useParams } from '@solidjs/router';
import { sessionList, type SessionSummary } from '../../ipc/dashboard';
import './panes.css';

export interface ChatPaneProps {
  /** Active workspace root. Sessions in the workspace match on this
   *  field; the dashboard sets it via the active-workspace signal. */
  workspaceRoot: string | null;
}

function shortId(id: string): string {
  return id.slice(0, 4);
}

export const ChatPane: Component<ChatPaneProps> = (props) => {
  const params = useParams<{ id?: string }>();
  const navigate = useNavigate();
  const [sessions, { refetch }] = createResource(sessionList);

  const rows = (): SessionSummary[] => {
    if (sessions.state !== 'ready') return [];
    const all = sessions() ?? [];
    if (!props.workspaceRoot) return [];
    return all.filter((s) => s.workspaceRoot === props.workspaceRoot);
  };

  const errorDetail = createMemo<string | null>(() => {
    const err = sessions.error;
    if (!err) return null;
    return err instanceof Error ? err.message : String(err);
  });

  const handleOpen = (id: string): void => {
    // No-op when clicking the active session (we're already there).
    if (id === params.id) return;
    navigate(`/session/${id}`);
  };

  return (
    <aside
      class="workspace-pane"
      aria-label="Sessions"
      data-testid="workspace-pane-chat"
    >
      <header class="workspace-pane__header">
        <span class="workspace-pane__title">SESSIONS</span>
        <button
          type="button"
          class="workspace-pane__action"
          data-testid="workspace-pane-chat-refresh"
          onClick={() => void refetch()}
          aria-label="Refresh sessions"
        >
          REFRESH
        </button>
      </header>

      <Show when={sessions.loading}>
        <div class="workspace-pane__loading" role="status">
          Loading sessions…
        </div>
      </Show>

      <Show when={!sessions.loading && errorDetail()}>
        {(detail) => (
          <div class="workspace-pane__error" role="alert">
            {detail()}
          </div>
        )}
      </Show>

      <Show when={!sessions.loading && !errorDetail()}>
        <Show
          when={rows().length > 0}
          fallback={
            <p
              class="workspace-pane__empty"
              data-testid="workspace-pane-chat-empty"
            >
              {props.workspaceRoot
                ? 'No sessions in this workspace yet. Start one from the dashboard.'
                : 'No workspace context — open a workspace from the dashboard.'}
            </p>
          }
        >
          <ul class="workspace-pane__list" role="list">
            <For each={rows()}>
              {(s) => {
                const active = (): boolean => s.id === params.id;
                return (
                  <li
                    role="listitem"
                    class="workspace-pane__row"
                    classList={{ 'workspace-pane__row--active': active() }}
                    data-testid={`workspace-pane-chat-row-${s.id}`}
                    aria-current={active() ? 'true' : undefined}
                    onClick={() => handleOpen(s.id)}
                  >
                    <span class="workspace-pane__row-name">{s.subject}</span>
                    <span class="workspace-pane__row-meta">#{shortId(s.id)}</span>
                  </li>
                );
              }}
            </For>
          </ul>
        </Show>
      </Show>
    </aside>
  );
};
