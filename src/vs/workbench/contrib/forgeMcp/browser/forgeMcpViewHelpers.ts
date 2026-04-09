/*---------------------------------------------------------------------------------------------
 * Forge - ForgeMcpView DOM helpers
 *--------------------------------------------------------------------------------------------*/

import { ForgeMcpServerStatusEntry } from '../../../services/forge/common/forgeMcpService.js';
import { ForgeMcpServerStatus } from '../../../services/forge/common/forgeMcpTypes.js';

export function getLocalityLabel(transport: 'local' | 'remote'): string {
	return transport;
}

export function createServerRow(server: ForgeMcpServerStatusEntry): HTMLElement {
	const row = document.createElement('div');
	row.className = 'forge-mcp-row';
	row.dataset['name'] = server.name;

	if (server.disabled) { row.classList.add('forge-mcp-row--disabled'); }
	if (server.status === ForgeMcpServerStatus.Error) { row.classList.add('forge-mcp-row--error'); }

	const dot = document.createElement('span');
	dot.className = `forge-mcp-status-dot forge-mcp-status-dot--${server.disabled ? 'disabled' : server.status.toLowerCase()}`;
	row.appendChild(dot);

	const name = document.createElement('span');
	name.className = 'forge-mcp-row-name';
	name.textContent = server.name;
	row.appendChild(name);

	const badge = document.createElement('span');
	badge.className = `forge-mcp-locality forge-mcp-locality--${server.transport}`;
	badge.textContent = getLocalityLabel(server.transport);
	row.appendChild(badge);

	const count = document.createElement('span');
	count.className = 'forge-mcp-tool-count';
	count.textContent = server.disabled ? 'disabled' : `${server.toolCount} tools`;
	row.appendChild(count);

	return row;
}

export function createToolRow(name: string, description: string): HTMLElement {
	const row = document.createElement('div');
	row.className = 'forge-mcp-tool-row';

	const toolName = document.createElement('div');
	toolName.className = 'forge-mcp-tool-name';
	toolName.textContent = name;
	row.appendChild(toolName);

	if (description) {
		const desc = document.createElement('div');
		desc.className = 'forge-mcp-tool-desc';
		desc.textContent = description;
		row.appendChild(desc);
	}

	return row;
}
