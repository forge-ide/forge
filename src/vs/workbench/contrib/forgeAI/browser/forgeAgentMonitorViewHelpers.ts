/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { ForgeAgentStatus, ForgeAgentTask } from '../../../services/forge/common/forgeAgentTypes.js';
import { AgentDefinition } from '../../../services/forge/common/forgeConfigResolutionTypes.js';

/** Maps ForgeAgentStatus enum → CSS class string for the status dot. */
export function getAgentStatusClass(status: ForgeAgentStatus): string {
	return status;
}

/** Stable sort: Running → Queued → Completed → MaxTurnsReached → Error. Does not mutate input. */
export function sortAgentsByStatus(agents: ForgeAgentTask[]): ForgeAgentTask[] {
	const order: Record<string, number> = {
		[ForgeAgentStatus.Running]: 0,
		[ForgeAgentStatus.Queued]: 1,
		[ForgeAgentStatus.Completed]: 2,
		[ForgeAgentStatus.MaxTurnsReached]: 3,
		[ForgeAgentStatus.Error]: 4,
	};
	return [...agents].sort((a, b) => (order[a.status] ?? 5) - (order[b.status] ?? 5));
}

/** Returns the empty-state element shown when no agent definitions are loaded. */
export function createEmptyDefinitionsState(): HTMLElement {
	const el = document.createElement('div');
	el.className = 'forge-agent-empty';
	el.textContent = 'No agent definitions found in .agents/';
	return el;
}

/** Returns the empty-state element shown when no agents are running or recent. */
export function createEmptyAgentsState(): HTMLElement {
	const el = document.createElement('div');
	el.className = 'forge-agent-empty';
	el.textContent = 'No agents running';
	return el;
}

/**
 * Creates a definition row element.
 * @param def - The agent definition to display.
 * @param disabled - Whether the agent is currently disabled.
 * @param onToggle - Called when the Enable/Disable button is clicked.
 */
export function createDefinitionRow(def: AgentDefinition, disabled: boolean, onToggle: () => void): HTMLElement {
	const row = document.createElement('div');
	row.className = 'forge-agent-def-row';
	if (disabled) {
		row.classList.add('disabled');
	}

	const name = document.createElement('span');
	name.className = 'forge-agent-def-name';
	name.textContent = def.name;
	if (disabled) {
		name.classList.add('disabled');
	}
	row.appendChild(name);

	const desc = document.createElement('span');
	desc.className = 'forge-agent-def-desc';
	desc.textContent = def.description || '(no description)';
	row.appendChild(desc);

	const toggle = document.createElement('button');
	toggle.className = 'forge-agent-def-toggle';
	toggle.title = disabled ? `Enable ${def.name}` : `Disable ${def.name}`;
	toggle.textContent = disabled ? 'Enable' : 'Disable';
	toggle.addEventListener('click', (e) => {
		e.stopPropagation();
		onToggle();
	});
	row.appendChild(toggle);

	return row;
}

/**
 * Creates an agent task row element.
 * @param agent - The agent task to display.
 * @param onCancel - Called with the agent id when the Cancel button is clicked.
 */
export function createAgentRow(agent: ForgeAgentTask, onCancel: (id: string) => void): HTMLElement {
	const row = document.createElement('div');
	row.className = 'forge-agent-row';

	const dot = document.createElement('span');
	dot.className = `forge-agent-status ${getAgentStatusClass(agent.status)}`;
	row.appendChild(dot);

	const name = document.createElement('span');
	name.className = 'forge-agent-row-name';
	name.textContent = agent.name;
	row.appendChild(name);

	const turns = document.createElement('span');
	turns.className = 'forge-agent-row-turns';
	turns.textContent = `${agent.currentTurn}/${agent.maxTurns}`;
	row.appendChild(turns);

	const steps = document.createElement('span');
	steps.className = 'forge-agent-row-steps';
	steps.textContent = `${agent.steps.length} steps`;
	row.appendChild(steps);

	if (agent.status === ForgeAgentStatus.Running) {
		const cancelBtn = document.createElement('button');
		cancelBtn.className = 'forge-agent-cancel-btn';
		cancelBtn.textContent = 'Cancel';
		cancelBtn.addEventListener('click', () => onCancel(agent.id));
		row.appendChild(cancelBtn);
	}

	return row;
}
