/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { IKeybindingService } from '../../../../platform/keybinding/common/keybinding.js';
import { IContextMenuService } from '../../../../platform/contextview/browser/contextView.js';
import { IConfigurationService } from '../../../../platform/configuration/common/configuration.js';
import { IContextKeyService } from '../../../../platform/contextkey/common/contextkey.js';
import { IInstantiationService } from '../../../../platform/instantiation/common/instantiation.js';
import { IViewDescriptorService } from '../../../common/views.js';
import { IOpenerService } from '../../../../platform/opener/common/opener.js';
import { IThemeService } from '../../../../platform/theme/common/themeService.js';
import { IHoverService } from '../../../../platform/hover/browser/hover.js';
import { ViewPane } from '../../../browser/parts/views/viewPane.js';
import { IViewletViewOptions } from '../../../browser/parts/views/viewsViewlet.js';
import { IForgeAgentService } from '../../../services/forge/common/forgeAgentService.js';
import { ForgeAgentTask, ForgeAgentStatus } from '../../../services/forge/common/forgeAgentTypes.js';
import { AgentDefinition } from '../../../services/forge/common/forgeConfigResolutionTypes.js';

export const FORGE_AGENT_MONITOR_VIEW_ID = 'workbench.forgeAI.agentMonitorView';

export class ForgeAgentMonitorView extends ViewPane {

	private listContainer!: HTMLElement;

	constructor(
		options: IViewletViewOptions,
		@IForgeAgentService private readonly agentService: IForgeAgentService,
		@IKeybindingService keybindingService: IKeybindingService,
		@IContextMenuService contextMenuService: IContextMenuService,
		@IConfigurationService configurationService: IConfigurationService,
		@IContextKeyService contextKeyService: IContextKeyService,
		@IViewDescriptorService viewDescriptorService: IViewDescriptorService,
		@IInstantiationService instantiationService: IInstantiationService,
		@IOpenerService openerService: IOpenerService,
		@IThemeService themeService: IThemeService,
		@IHoverService hoverService: IHoverService,
	) {
		super(options, keybindingService, contextMenuService, configurationService, contextKeyService, viewDescriptorService, instantiationService, openerService, themeService, hoverService);
	}

	protected override renderBody(container: HTMLElement): void {
		super.renderBody(container);
		container.classList.add('forge-agent-monitor-view');

		this.listContainer = document.createElement('div');
		this.listContainer.className = 'forge-agent-list';
		container.appendChild(this.listContainer);

		this._register(this.agentService.onDidChangeAgent(() => {
			this.renderAll();
		}));

		this.renderAll();
	}

	private renderAll(): void {
		this.listContainer.innerHTML = '';

		// Section 1: Available Agent Definitions
		const defsHeader = document.createElement('div');
		defsHeader.className = 'forge-agent-section-header';
		defsHeader.textContent = 'Available Agents';
		this.listContainer.appendChild(defsHeader);

		const definitions = this.agentService.getAvailableDefinitions();
		if (definitions.length === 0) {
			const empty = document.createElement('div');
			empty.className = 'forge-agent-empty';
			empty.textContent = 'No agent definitions found in .agents/';
			this.listContainer.appendChild(empty);
		} else {
			for (const def of definitions) {
				this.listContainer.appendChild(this.renderDefinitionRow(def));
			}
		}

		// Section 2: Running/Recent Agents
		const runHeader = document.createElement('div');
		runHeader.className = 'forge-agent-section-header';
		runHeader.textContent = 'Running & Recent';
		this.listContainer.appendChild(runHeader);

		const agents = this.agentService.getAllAgents();
		if (agents.length === 0) {
			const empty = document.createElement('div');
			empty.className = 'forge-agent-empty';
			empty.textContent = 'No agents running';
			this.listContainer.appendChild(empty);
		} else {
			const sorted = [...agents].sort((a, b) => {
				const order: Record<string, number> = {
					[ForgeAgentStatus.Running]: 0,
					[ForgeAgentStatus.Queued]: 1,
					[ForgeAgentStatus.Completed]: 2,
					[ForgeAgentStatus.MaxTurnsReached]: 3,
					[ForgeAgentStatus.Error]: 4
				};
				return (order[a.status] ?? 5) - (order[b.status] ?? 5);
			});

			for (const agent of sorted) {
				this.listContainer.appendChild(this.renderAgentRow(agent));
			}
		}
	}

	private renderDefinitionRow(def: AgentDefinition): HTMLElement {
		const row = document.createElement('div');
		row.className = 'forge-agent-def-row';
		const isDisabled = this.agentService.isAgentDisabled(def.name);
		if (isDisabled) {
			row.classList.add('disabled');
		}

		const name = document.createElement('span');
		name.className = 'forge-agent-def-name';
		name.textContent = def.name;
		if (isDisabled) {
			name.classList.add('disabled');
		}
		row.appendChild(name);

		const desc = document.createElement('span');
		desc.className = 'forge-agent-def-desc';
		desc.textContent = def.description || '(no description)';
		row.appendChild(desc);

		const toggle = document.createElement('button');
		toggle.className = 'forge-agent-def-toggle';
		toggle.title = isDisabled ? `Enable ${def.name}` : `Disable ${def.name}`;
		toggle.textContent = isDisabled ? 'Enable' : 'Disable';
		toggle.addEventListener('click', (e) => {
			e.stopPropagation();
			this.agentService.toggleAgentDisabled(def.name, !isDisabled);
		});
		row.appendChild(toggle);

		return row;
	}

	private renderAgentRow(agent: ForgeAgentTask): HTMLElement {
		const row = document.createElement('div');
		row.className = 'forge-agent-row';

		const dot = document.createElement('span');
		dot.className = `forge-agent-status ${agent.status}`;
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
			cancelBtn.addEventListener('click', () => {
				this.agentService.cancelAgent(agent.id);
			});
			row.appendChild(cancelBtn);
		}

		return row;
	}

	protected override layoutBody(height: number, width: number): void {
		super.layoutBody(height, width);
	}
}
