import type { Component } from 'solid-js';
import { listMcpServers, SESSION_WIDE_SCOPE } from '../../ipc/catalog';
import { RosterPane } from './RosterPane';

export interface McpPaneProps {
  workspaceRoot: string | null;
}

/** Workspace MCP servers pane. Reads from the on-disk `.mcp.json` —
 *  distinct from the running-daemon view at `session_list_mcp_servers`,
 *  which surfaces lifecycle state. The Catalog page is where toggle /
 *  add / remove actions live. */
export const McpPane: Component<McpPaneProps> = (props) => (
  <RosterPane
    title="MCP SERVERS"
    slug="mcp"
    workspaceRoot={props.workspaceRoot}
    fetcher={(ws) => listMcpServers(ws, SESSION_WIDE_SCOPE)}
    catalogRoute="/catalog/mcp"
    emptyMessage="No MCP servers configured."
    rowFor={(entry) => {
      if (entry.entry.type !== 'Mcp') return null;
      return { id: entry.entry.id };
    }}
  />
);
