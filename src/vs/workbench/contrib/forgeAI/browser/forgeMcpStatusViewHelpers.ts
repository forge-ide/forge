/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { ForgeMcpServerStatus } from '../../../services/forge/common/forgeMcpTypes.js';
import { ForgeMcpServerStatusEntry } from '../../../services/forge/common/forgeMcpService.js';

/** Maps (status, disabled) → CSS class string for the status dot. */
export function getServerStatusClass(status: ForgeMcpServerStatus, disabled: boolean): string {
	if (disabled) {
		return 'disabled';
	}
	switch (status) {
		case ForgeMcpServerStatus.Connected: return 'connected';
		case ForgeMcpServerStatus.Connecting: return 'connecting';
		case ForgeMcpServerStatus.Error: return 'error';
		default: return 'disconnected';
	}
}

/** Returns "disabled" when disabled=true, otherwise "<n> tools". */
export function getToolCountText(toolCount: number, disabled: boolean): string {
	return disabled ? 'disabled' : `${toolCount} tools`;
}

/** Returns the empty-state element shown when no MCP servers are configured. */
export function createEmptyServersState(): HTMLElement {
	const el = document.createElement('div');
	el.className = 'forge-mcp-empty';
	el.textContent = 'No MCP servers configured';
	return el;
}

/**
 * Creates a server row element.
 * @param server - The server status entry to display.
 * @param onToggle - Called when the Enable/Disable button is clicked.
 */
export function createServerRow(server: ForgeMcpServerStatusEntry, onToggle: () => void): HTMLElement {
	const row = document.createElement('div');
	row.className = 'forge-mcp-server-row';
	if (server.disabled) {
		row.classList.add('disabled');
	}

	const dot = document.createElement('span');
	dot.className = 'forge-mcp-server-dot';
	dot.classList.add(getServerStatusClass(server.status, server.disabled));
	row.appendChild(dot);

	const name = document.createElement('span');
	name.className = 'forge-mcp-server-name';
	name.textContent = server.name;
	if (server.disabled) {
		name.classList.add('disabled');
	}
	row.appendChild(name);

	const toolCount = document.createElement('span');
	toolCount.className = 'forge-mcp-tool-count';
	toolCount.textContent = getToolCountText(server.toolCount, server.disabled);
	row.appendChild(toolCount);

	const toggle = document.createElement('button');
	toggle.className = 'forge-mcp-server-toggle';
	toggle.title = server.disabled ? `Enable ${server.name}` : `Disable ${server.name}`;
	toggle.textContent = server.disabled ? 'Enable' : 'Disable';
	toggle.addEventListener('click', (e) => {
		e.stopPropagation();
		onToggle();
	});
	row.appendChild(toggle);

	return row;
}
