/*---------------------------------------------------------------------------------------------
 * Forge - ForgeAgentsView DOM helpers
 *--------------------------------------------------------------------------------------------*/

import { AgentDefinition } from '../../../services/forge/common/forgeConfigResolutionTypes.js';
import { ForgeAgentStatus } from '../../../services/forge/common/forgeAgentTypes.js';

export function getAgentStatusLabel(status: ForgeAgentStatus): string {
	switch (status) {
		case ForgeAgentStatus.Running: return 'Running';
		case ForgeAgentStatus.Queued: return 'Queued';
		case ForgeAgentStatus.Completed: return 'Completed';
		case ForgeAgentStatus.MaxTurnsReached: return 'Max turns reached';
		case ForgeAgentStatus.Error: return 'Error';
		default: return 'Unknown';
	}
}

export function createDefinitionRow(def: AgentDefinition, isRunning: boolean): HTMLElement {
	const row = document.createElement('div');
	row.className = 'forge-agent-row';
	row.dataset['name'] = def.name;

	if (isRunning) {
		const dot = document.createElement('span');
		dot.className = 'forge-agent-live-dot';
		row.appendChild(dot);
	}

	const label = document.createElement('span');
	label.className = 'forge-agent-row-name';
	label.textContent = def.name;
	row.appendChild(label);

	return row;
}

export function createRunningAgentRow(
	agentId: string,
	name: string,
	status: ForgeAgentStatus,
	turn: number,
	maxTurns: number,
): HTMLElement {
	const row = document.createElement('div');
	row.className = 'forge-agent-row forge-agent-row--running';
	row.dataset['agentId'] = agentId;

	const label = document.createElement('span');
	label.className = 'forge-agent-row-name';
	label.textContent = name;
	row.appendChild(label);

	const meta = document.createElement('span');
	meta.className = 'forge-agent-row-meta';
	meta.textContent = `${getAgentStatusLabel(status)} · Turn ${turn} / ${maxTurns}`;
	row.appendChild(meta);

	return row;
}

export function createChipElement(label: string, onRemove: () => void): HTMLElement {
	const chip = document.createElement('div');
	chip.className = 'forge-chip';
	chip.dataset['label'] = label;

	const text = document.createElement('span');
	text.className = 'forge-chip-label';
	text.textContent = label;
	chip.appendChild(text);

	const remove = document.createElement('button');
	remove.className = 'forge-chip-remove';
	remove.textContent = 'x';
	remove.title = `Remove ${label}`;
	remove.addEventListener('click', (e) => {
		e.stopPropagation();
		onRemove();
	});
	chip.appendChild(remove);

	return chip;
}

export function createAddChipButton(onClick: () => void): HTMLElement {
	const btn = document.createElement('button');
	btn.className = 'forge-chip forge-chip--add';
	btn.textContent = '+ Add';
	btn.addEventListener('click', onClick);
	return btn;
}
