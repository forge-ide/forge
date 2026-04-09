/*---------------------------------------------------------------------------------------------
 * Forge - ForgeAgentsView (split-pane: definitions list + detail panel)
 *--------------------------------------------------------------------------------------------*/

import { reset, getWindow } from '../../../../base/browser/dom.js';
import { IKeybindingService } from '../../../../platform/keybinding/common/keybinding.js';
import { IContextMenuService } from '../../../../platform/contextview/browser/contextView.js';
import { IConfigurationService } from '../../../../platform/configuration/common/configuration.js';
import { IContextKeyService } from '../../../../platform/contextkey/common/contextkey.js';
import { IInstantiationService } from '../../../../platform/instantiation/common/instantiation.js';
import { IOpenerService } from '../../../../platform/opener/common/opener.js';
import { IThemeService } from '../../../../platform/theme/common/themeService.js';
import { IHoverService } from '../../../../platform/hover/browser/hover.js';
import { URI } from '../../../../base/common/uri.js';
import { ViewPane } from '../../../browser/parts/views/viewPane.js';
import { IViewletViewOptions } from '../../../browser/parts/views/viewsViewlet.js';
import { IViewDescriptorService } from '../../../common/views.js';
import { IForgeAgentService } from '../../../services/forge/common/forgeAgentService.js';
import { IForgeSessionContextService } from '../../../services/forge/common/forgeSessionContextService.js';
import { IForgeSkillService } from '../../../services/forge/common/forgeSkillService.js';
import { IForgeMcpService } from '../../../services/forge/common/forgeMcpService.js';
import { ForgeAgentTask, ForgeAgentStatus } from '../../../services/forge/common/forgeAgentTypes.js';
import { AgentDefinition } from '../../../services/forge/common/forgeConfigResolutionTypes.js';
import {
	createDefinitionRow,
	createRunningAgentRow,
	createChipElement,
	createAddChipButton,
	getAgentStatusLabel,
} from './forgeAgentsViewHelpers.js';

export const FORGE_AGENTS_VIEW_ID = 'workbench.forgeAgents.mainView';

const isActiveStatus = (s: ForgeAgentStatus) =>
	s === ForgeAgentStatus.Running || s === ForgeAgentStatus.Queued;

export class ForgeAgentsView extends ViewPane {

	private splitLeft!: HTMLElement;
	private splitRight!: HTMLElement;
	private selectedDefName: string | undefined;
	private selectedAgentId: string | undefined;

	constructor(
		options: IViewletViewOptions,
		@IForgeAgentService private readonly agentService: IForgeAgentService,
		@IForgeSessionContextService private readonly sessionContextService: IForgeSessionContextService,
		@IForgeSkillService private readonly skillService: IForgeSkillService,
		@IForgeMcpService private readonly mcpService: IForgeMcpService,
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
		container.classList.add('forge-agents-view');

		this.splitLeft = document.createElement('div');
		this.splitLeft.className = 'forge-split-left';
		container.appendChild(this.splitLeft);

		this.splitRight = document.createElement('div');
		this.splitRight.className = 'forge-split-right forge-split-right--hidden';
		container.appendChild(this.splitRight);

		this._register(this.agentService.onDidChangeAgent(() => this._renderList()));

		this._renderList();
		this._renderEmptyDetail();
	}

	private _renderList(): void {
		reset(this.splitLeft);

		const defsLabel = document.createElement('div');
		defsLabel.className = 'forge-section-label';
		defsLabel.textContent = 'Definitions';
		this.splitLeft.appendChild(defsLabel);

		const definitions = this.agentService.getAvailableDefinitions();
		for (const def of definitions) {
			const isRunning = this.agentService.getRunningAgents().some(a => a.name === def.name);
			const row = createDefinitionRow(def, isRunning);
			if (this.selectedDefName === def.name && !this.selectedAgentId) {
				row.classList.add('forge-row--selected');
			}
			row.addEventListener('click', () => this._selectDefinition(def));
			this.splitLeft.appendChild(row);
		}

		const runningAgents = this.agentService.getRunningAgents();
		if (runningAgents.length > 0) {
			const runLabel = document.createElement('div');
			runLabel.className = 'forge-section-label';
			runLabel.textContent = 'Running';
			this.splitLeft.appendChild(runLabel);

			for (const agent of runningAgents) {
				const row = createRunningAgentRow(agent.id, agent.name, agent.status, agent.currentTurn, agent.maxTurns);
				if (this.selectedAgentId === agent.id) {
					row.classList.add('forge-row--selected');
				}
				row.addEventListener('click', () => this._selectAgent(agent));
				this.splitLeft.appendChild(row);
			}
		}
	}

	private _selectDefinition(def: AgentDefinition): void {
		this.selectedDefName = def.name;
		this.selectedAgentId = undefined;
		this._renderList();
		this._renderDefinitionDetail(def);
	}

	private _selectAgent(agent: ForgeAgentTask): void {
		this.selectedAgentId = agent.id;
		this.selectedDefName = undefined;
		this._renderList();
		this._renderAgentDetail(agent);
	}

	private _renderEmptyDetail(): void {
		reset(this.splitRight);
		this.splitRight.classList.add('forge-split-right--hidden');
	}

	private _renderDefinitionDetail(def: AgentDefinition): void {
		this.splitRight.classList.remove('forge-split-right--hidden');
		reset(this.splitRight);

		const title = document.createElement('div');
		title.className = 'forge-detail-title';
		title.textContent = def.name;
		this.splitRight.appendChild(title);

		if (def.description) {
			const desc = document.createElement('div');
			desc.className = 'forge-detail-desc';
			desc.textContent = def.description;
			this.splitRight.appendChild(desc);
		}

		this.splitRight.appendChild(this._renderReadonlyChips('Skills', def.skills ?? []));
		this.splitRight.appendChild(this._renderReadonlyChips('MCP Servers', def.mcpServers ?? []));

		const actions = document.createElement('div');
		actions.className = 'forge-detail-actions';

		if (def.sourcePath) {
			const editBtn = document.createElement('button');
			editBtn.className = 'forge-btn forge-btn--neutral';
			editBtn.textContent = 'Edit Agent';
			editBtn.addEventListener('click', () => {
				this.openerService.open(URI.file(def.sourcePath!));
			});
			actions.appendChild(editBtn);
		}

		this.splitRight.appendChild(actions);
	}

	private _renderAgentDetail(agent: ForgeAgentTask): void {
		this.splitRight.classList.remove('forge-split-right--hidden');
		reset(this.splitRight);

		const title = document.createElement('div');
		title.className = 'forge-detail-title';
		title.textContent = agent.name;
		this.splitRight.appendChild(title);

		const statusEl = document.createElement('div');
		statusEl.className = 'forge-detail-status';
		statusEl.textContent = `${getAgentStatusLabel(agent.status)} · Turn ${agent.currentTurn} / ${agent.maxTurns}`;
		this.splitRight.appendChild(statusEl);

		if (agent.taskDescription) {
			const task = document.createElement('div');
			task.className = 'forge-detail-task';
			task.textContent = agent.taskDescription;
			this.splitRight.appendChild(task);
		}

		const ctx = this.sessionContextService.getContext(agent.id);

		this.splitRight.appendChild(this._renderMutableChips(
			'Skills',
			ctx.activeSkills,
			() => this._openPicker(agent.id, 'skills'),
			(skill) => {
				const updated = ctx.activeSkills.filter(s => s !== skill);
				this.sessionContextService.setSkills(agent.id, updated);
				this._renderAgentDetail(this.agentService.getAgent(agent.id) ?? agent);
			}
		));

		this.splitRight.appendChild(this._renderMutableChips(
			'MCP Servers',
			ctx.activeMcpServers,
			() => this._openPicker(agent.id, 'mcp'),
			(server) => {
				const updated = ctx.activeMcpServers.filter(s => s !== server);
				this.sessionContextService.setMcpServers(agent.id, updated);
				this._renderAgentDetail(this.agentService.getAgent(agent.id) ?? agent);
			}
		));

		if (isActiveStatus(agent.status)) {
			const actions = document.createElement('div');
			actions.className = 'forge-detail-actions';

			const cancelBtn = document.createElement('button');
			cancelBtn.className = 'forge-btn forge-btn--danger';
			cancelBtn.textContent = 'Cancel';
			cancelBtn.addEventListener('click', () => {
				this.agentService.cancelAgent(agent.id);
				this.selectedAgentId = undefined;
				this._renderList();
				this._renderEmptyDetail();
			});
			actions.appendChild(cancelBtn);
			this.splitRight.appendChild(actions);
		}
	}

	private _renderReadonlyChips(sectionLabel: string, items: string[]): HTMLElement {
		const section = document.createElement('div');
		section.className = 'forge-chip-section';

		const label = document.createElement('div');
		label.className = 'forge-section-label';
		label.style.padding = '4px 0 2px';
		label.textContent = sectionLabel;
		section.appendChild(label);

		const row = document.createElement('div');
		row.className = 'forge-chip-row';

		if (items.length === 0) {
			const none = document.createElement('span');
			none.className = 'forge-detail-empty';
			none.style.padding = '0';
			none.textContent = 'None';
			row.appendChild(none);
		} else {
			for (const item of items) {
				const chip = document.createElement('div');
				chip.className = 'forge-chip';
				chip.textContent = item;
				row.appendChild(chip);
			}
		}

		section.appendChild(row);
		return section;
	}

	private _renderMutableChips(
		sectionLabel: string,
		items: string[],
		onAdd: () => void,
		onRemove: (item: string) => void,
	): HTMLElement {
		const section = document.createElement('div');
		section.className = 'forge-chip-section';

		const label = document.createElement('div');
		label.className = 'forge-section-label';
		label.style.padding = '4px 0 2px';
		label.textContent = sectionLabel;
		section.appendChild(label);

		const row = document.createElement('div');
		row.className = 'forge-chip-row';

		for (const item of items) {
			row.appendChild(createChipElement(item, () => onRemove(item)));
		}

		row.appendChild(createAddChipButton(onAdd));
		section.appendChild(row);
		return section;
	}

	private _openPicker(sessionId: string, type: 'skills' | 'mcp'): void {
		const ctx = this.sessionContextService.getContext(sessionId);
		const active = type === 'skills' ? ctx.activeSkills : ctx.activeMcpServers;

		const candidates: string[] = type === 'skills'
			? this.skillService.getAvailableSkills().map(s => s.name)
			: this.mcpService.getServerStatuses().map(s => s.name);

		const popover = document.createElement('div');
		popover.className = 'forge-picker-popover';

		const closePopover = () => popover.remove();

		for (const name of candidates) {
			const pickRow = document.createElement('div');
			pickRow.className = 'forge-picker-row';

			const checkbox = document.createElement('input');
			checkbox.type = 'checkbox';
			checkbox.checked = active.includes(name);
			checkbox.addEventListener('change', () => {
				const currentCtx = this.sessionContextService.getContext(sessionId);
				const currentActive = type === 'skills' ? currentCtx.activeSkills : currentCtx.activeMcpServers;
				const updated = checkbox.checked
					? [...currentActive, name]
					: currentActive.filter(s => s !== name);

				if (type === 'skills') {
					this.sessionContextService.setSkills(sessionId, updated);
				} else {
					this.sessionContextService.setMcpServers(sessionId, updated);
				}

				const agent = this.agentService.getAgent(sessionId);
				if (agent) {
					closePopover();
					this._renderAgentDetail(agent);
				}
			});

			const nameSpan = document.createElement('span');
			nameSpan.textContent = name;

			pickRow.appendChild(checkbox);
			pickRow.appendChild(nameSpan);
			popover.appendChild(pickRow);
		}

		if (candidates.length === 0) {
			const empty = document.createElement('div');
			empty.className = 'forge-picker-row';
			empty.textContent = 'None available';
			popover.appendChild(empty);
		}

		getWindow(this.splitRight).document.addEventListener('click', closePopover, { once: true, capture: true });
		this.splitRight.appendChild(popover);
	}

	protected override layoutBody(height: number, width: number): void {
		super.layoutBody(height, width);
	}
}
