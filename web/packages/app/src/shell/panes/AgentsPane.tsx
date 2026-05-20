import type { Component } from 'solid-js';
import { listAgents, SESSION_WIDE_SCOPE } from '../../ipc/catalog';
import { RosterPane } from './RosterPane';

export interface AgentsPaneProps {
  workspaceRoot: string | null;
}

/** Workspace Agents pane — thin wrapper around the shared roster chrome.
 *  Surfaces agent defs loaded from `.agents/*.md` for the active
 *  workspace plus the user-scope roster. Click "Manage" → Catalog. */
export const AgentsPane: Component<AgentsPaneProps> = (props) => (
  <RosterPane
    title="AGENTS"
    slug="agents"
    workspaceRoot={props.workspaceRoot}
    fetcher={(ws) => listAgents(ws, SESSION_WIDE_SCOPE)}
    catalogRoute="/catalog/agents"
    emptyMessage="No agents defined yet."
    rowFor={(entry) => {
      if (entry.entry.type !== 'Agent') return null;
      return { id: entry.entry.id };
    }}
  />
);
