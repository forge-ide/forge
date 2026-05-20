// Shared chrome for the Agents / Skills / MCP panes.
//
// Each of those three surfaces the same shape on disk: a workspace-scoped
// roster (`list_agents` / `list_skills` / `list_mcp_servers`) returning
// `ScopedRosterEntry[]`. The Catalog page already renders the rich
// (toggle / enable / detail) experience for the same data; this sidebar
// pane is the lightweight glance — "what's loaded for this workspace,
// quick" — with a deep-link into Catalog for management.

import {
  type Component,
  For,
  Show,
  createMemo,
  createResource,
} from 'solid-js';
import { useNavigate } from '@solidjs/router';
import type { ScopedRosterEntry } from '@forge/ipc';
import './panes.css';

export interface RosterPaneProps {
  /** Pane title in the header chrome (e.g. "AGENTS"). */
  title: string;
  /** Test-id slug for the wrapper element and its children. */
  slug: string;
  /** Active workspace root. `null` short-circuits the fetch and renders
   *  the no-workspace empty state. */
  workspaceRoot: string | null;
  /** Fetcher — usually one of `listAgents` / `listSkills` /
   *  `listMcpServers` curried with `SESSION_WIDE_SCOPE`. */
  fetcher: (workspaceRoot: string) => Promise<ScopedRosterEntry[]>;
  /** Catalog route to link into for management actions (e.g.
   *  `/catalog/agents`). The "manage" affordance navigates here. */
  catalogRoute: string;
  /** Empty-state copy when the roster is loaded but empty. */
  emptyMessage: string;
  /** Project a roster entry to a (id, badge?) for rendering. Lets each
   *  pane render the variant fields its kind cares about — e.g. the
   *  MCP pane could surface a `(workspace|user)` scope hint later. */
  rowFor: (entry: ScopedRosterEntry) => { id: string; meta?: string } | null;
}

export const RosterPane: Component<RosterPaneProps> = (props) => {
  const navigate = useNavigate();
  const [roster, { refetch }] = createResource<
    ScopedRosterEntry[] | null,
    string | null
  >(
    () => props.workspaceRoot,
    async (ws): Promise<ScopedRosterEntry[] | null> => {
      if (!ws) return null;
      return props.fetcher(ws);
    },
  );

  const rows = createMemo(() => {
    if (roster.state !== 'ready') return [];
    const data = roster();
    if (!data) return [];
    const out: { id: string; meta?: string; key: string }[] = [];
    data.forEach((e, i) => {
      const row = props.rowFor(e);
      if (row) out.push({ ...row, key: `${row.id}-${i}` });
    });
    return out;
  });

  const errorDetail = createMemo<string | null>(() => {
    const err = roster.error;
    if (!err) return null;
    return err instanceof Error ? err.message : String(err);
  });

  return (
    <aside
      class="workspace-pane"
      aria-label={props.title}
      data-testid={`workspace-pane-${props.slug}`}
    >
      <header class="workspace-pane__header">
        <span class="workspace-pane__title">{props.title}</span>
        <button
          type="button"
          class="workspace-pane__action"
          data-testid={`workspace-pane-${props.slug}-manage`}
          onClick={() => navigate(props.catalogRoute)}
          aria-label={`Manage ${props.title.toLowerCase()}`}
        >
          MANAGE
        </button>
      </header>

      <Show when={!props.workspaceRoot}>
        <p
          class="workspace-pane__empty"
          data-testid={`workspace-pane-${props.slug}-no-workspace`}
        >
          No workspace open.
        </p>
      </Show>

      <Show when={props.workspaceRoot && roster.loading}>
        <div class="workspace-pane__loading" role="status">
          Loading…
        </div>
      </Show>

      <Show when={props.workspaceRoot && !roster.loading && errorDetail()}>
        {(detail) => (
          <div class="workspace-pane__error" role="alert">
            <p>{detail()}</p>
            <button
              type="button"
              class="workspace-pane__action"
              onClick={() => void refetch()}
            >
              RETRY
            </button>
          </div>
        )}
      </Show>

      <Show
        when={
          props.workspaceRoot && !roster.loading && !errorDetail()
        }
      >
        <Show
          when={rows().length > 0}
          fallback={
            <p
              class="workspace-pane__empty"
              data-testid={`workspace-pane-${props.slug}-empty`}
            >
              {props.emptyMessage}
            </p>
          }
        >
          <ul class="workspace-pane__list" role="list">
            <For each={rows()}>
              {(row) => (
                <li
                  role="listitem"
                  class="workspace-pane__row"
                  data-testid={`workspace-pane-${props.slug}-row-${row.id}`}
                >
                  <span class="workspace-pane__row-name">{row.id}</span>
                  <Show when={row.meta}>
                    <span class="workspace-pane__row-meta">{row.meta}</span>
                  </Show>
                </li>
              )}
            </For>
          </ul>
        </Show>
      </Show>
    </aside>
  );
};
