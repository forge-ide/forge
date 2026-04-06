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
import { reset } from '../../../../base/browser/dom.js';
import { ViewPane } from '../../../browser/parts/views/viewPane.js';
import { IViewletViewOptions } from '../../../browser/parts/views/viewsViewlet.js';
import { IForgeAgentService } from '../../../services/forge/common/forgeAgentService.js';
import { ForgeAgentTask } from '../../../services/forge/common/forgeAgentTypes.js';
import { AgentDefinition } from '../../../services/forge/common/forgeConfigResolutionTypes.js';
import {
	sortAgentsByStatus,
	createEmptyDefinitionsState,
	createEmptyAgentsState,
	createDefinitionRow,
	createAgentRow,
} from './forgeAgentMonitorViewHelpers.js';

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
		reset(this.listContainer);

		// Section 1: Available Agent Definitions
		const defsHeader = document.createElement('div');
		defsHeader.className = 'forge-agent-section-header';
		defsHeader.textContent = 'Available Agents';
		this.listContainer.appendChild(defsHeader);

		const definitions = this.agentService.getAvailableDefinitions();
		if (definitions.length === 0) {
			this.listContainer.appendChild(createEmptyDefinitionsState());
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
			this.listContainer.appendChild(createEmptyAgentsState());
		} else {
			for (const agent of sortAgentsByStatus(agents)) {
				this.listContainer.appendChild(this.renderAgentRow(agent));
			}
		}
	}

	private renderDefinitionRow(def: AgentDefinition): HTMLElement {
		const isDisabled = this.agentService.isAgentDisabled(def.name);
		return createDefinitionRow(def, isDisabled, () => this.agentService.toggleAgentDisabled(def.name, !isDisabled));
	}

	private renderAgentRow(agent: ForgeAgentTask): HTMLElement {
		return createAgentRow(agent, (id) => this.agentService.cancelAgent(id));
	}

	protected override layoutBody(height: number, width: number): void {
		super.layoutBody(height, width);
	}
}
