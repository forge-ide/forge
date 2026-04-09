/*---------------------------------------------------------------------------------------------
 * Forge - ForgeMcpView (split-pane: server list + detail panel)
 *--------------------------------------------------------------------------------------------*/

import { reset } from '../../../../base/browser/dom.js';
import { IKeybindingService } from '../../../../platform/keybinding/common/keybinding.js';
import { IContextMenuService } from '../../../../platform/contextview/browser/contextView.js';
import { IConfigurationService } from '../../../../platform/configuration/common/configuration.js';
import { IContextKeyService } from '../../../../platform/contextkey/common/contextkey.js';
import { IInstantiationService } from '../../../../platform/instantiation/common/instantiation.js';
import { IOpenerService } from '../../../../platform/opener/common/opener.js';
import { IThemeService } from '../../../../platform/theme/common/themeService.js';
import { IHoverService } from '../../../../platform/hover/browser/hover.js';
import { ViewPane } from '../../../browser/parts/views/viewPane.js';
import { IViewletViewOptions } from '../../../browser/parts/views/viewsViewlet.js';
import { IViewDescriptorService } from '../../../common/views.js';
import { IForgeMcpService, ForgeMcpServerStatusEntry } from '../../../services/forge/common/forgeMcpService.js';
import { AIToolDefinition } from '../../../../platform/ai/common/aiProvider.js';
import { createServerRow, createToolRow } from './forgeMcpViewHelpers.js';

export const FORGE_MCP_VIEW_ID = 'workbench.forgeMcp.mainView';

export class ForgeMcpView extends ViewPane {

	private splitLeft!: HTMLElement;
	private splitRight!: HTMLElement;
	private selectedServerName: string | undefined;

	constructor(
		options: IViewletViewOptions,
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
		container.classList.add('forge-mcp-view');

		this.splitLeft = document.createElement('div');
		this.splitLeft.className = 'forge-split-left';
		container.appendChild(this.splitLeft);

		this.splitRight = document.createElement('div');
		this.splitRight.className = 'forge-split-right';
		container.appendChild(this.splitRight);

		this._register(this.mcpService.onDidChangeServerStatus(() => this._renderList()));
		this._register(this.mcpService.onDidChangeTools(() => this._renderList()));

		this._renderList();
		this._renderEmptyDetail();
	}

	private _renderList(): void {
		reset(this.splitLeft);

		const label = document.createElement('div');
		label.className = 'forge-section-label';
		label.textContent = 'MCP Servers';
		this.splitLeft.appendChild(label);

		const servers = this.mcpService.getServerStatuses();

		if (servers.length === 0) {
			const empty = document.createElement('div');
			empty.className = 'forge-detail-empty';
			empty.textContent = 'No MCP servers configured.';
			this.splitLeft.appendChild(empty);
			return;
		}

		for (const server of servers) {
			const row = createServerRow(server);
			if (this.selectedServerName === server.name) {
				row.classList.add('forge-row--selected');
			}
			row.addEventListener('click', () => this._selectServer(server));
			this.splitLeft.appendChild(row);
		}
	}

	private _selectServer(server: ForgeMcpServerStatusEntry): void {
		this.selectedServerName = server.name;
		this._renderList();
		this._renderServerDetail(server);
	}

	private _renderEmptyDetail(): void {
		reset(this.splitRight);
		const empty = document.createElement('div');
		empty.className = 'forge-detail-empty';
		empty.textContent = 'Select an MCP server to view details.';
		this.splitRight.appendChild(empty);
	}

	private _renderServerDetail(server: ForgeMcpServerStatusEntry): void {
		reset(this.splitRight);

		const header = document.createElement('div');
		header.className = 'forge-detail-header';

		const title = document.createElement('div');
		title.className = 'forge-detail-title';
		title.textContent = server.name;
		header.appendChild(title);

		const badge = document.createElement('span');
		badge.className = `forge-mcp-locality forge-mcp-locality--${server.transport}`;
		badge.textContent = server.transport;
		header.appendChild(badge);

		this.splitRight.appendChild(header);

		const statusEl = document.createElement('div');
		statusEl.className = 'forge-detail-status';
		statusEl.textContent = server.disabled ? 'disabled' : `${server.status} · ${server.toolCount} tools`;
		this.splitRight.appendChild(statusEl);

		const actions = document.createElement('div');
		actions.className = 'forge-detail-actions';

		const toggleBtn = document.createElement('button');
		toggleBtn.className = server.disabled ? 'forge-btn forge-btn--neutral' : 'forge-btn forge-btn--danger';
		toggleBtn.textContent = server.disabled ? 'Enable' : 'Disable';
		toggleBtn.addEventListener('click', () => {
			this.mcpService.toggleServerDisabled(server.name, !server.disabled);
		});
		actions.appendChild(toggleBtn);
		this.splitRight.appendChild(actions);

		const toolsLabel = document.createElement('div');
		toolsLabel.className = 'forge-section-label';
		toolsLabel.style.padding = '8px 0 3px';
		toolsLabel.textContent = 'Tools';
		this.splitRight.appendChild(toolsLabel);

		this.mcpService.listTools().then((tools: AIToolDefinition[]) => {
			if (tools.length === 0) {
				const noTools = document.createElement('div');
				noTools.className = 'forge-detail-empty';
				noTools.textContent = 'No tools available.';
				this.splitRight.appendChild(noTools);
				return;
			}

			const toolList = document.createElement('div');
			toolList.className = 'forge-mcp-tool-list';
			for (const tool of tools) {
				toolList.appendChild(createToolRow(tool.name, tool.description));
			}
			this.splitRight.appendChild(toolList);
		}).catch(() => {
			const errEl = document.createElement('div');
			errEl.className = 'forge-mcp-error-msg';
			errEl.textContent = 'Failed to load tools.';
			this.splitRight.appendChild(errEl);
		});
	}

	protected override layoutBody(height: number, width: number): void {
		super.layoutBody(height, width);
	}
}
