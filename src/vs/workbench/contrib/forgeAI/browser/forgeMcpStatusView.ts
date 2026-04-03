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
import { ITelemetryService } from '../../../../platform/telemetry/common/telemetry.js';
import { IHoverService } from '../../../../platform/hover/browser/hover.js';
import { ViewPane } from '../../../browser/parts/views/viewPane.js';
import { IViewletViewOptions } from '../../../browser/parts/views/viewsViewlet.js';
import { IForgeMcpService, ForgeMcpServerStatusEntry } from '../../../services/forge/common/forgeMcpService.js';
import { ForgeMcpServerStatus } from '../../../services/forge/common/forgeMcpTypes.js';

export const FORGE_MCP_STATUS_VIEW_ID = 'workbench.forgeAI.mcpStatusView';

export class ForgeMcpStatusView extends ViewPane {

	private listContainer!: HTMLElement;

	constructor(
		options: IViewletViewOptions,
		@IForgeMcpService private readonly forgeMcpService: IForgeMcpService,
		@IKeybindingService keybindingService: IKeybindingService,
		@IContextMenuService contextMenuService: IContextMenuService,
		@IConfigurationService configurationService: IConfigurationService,
		@IContextKeyService contextKeyService: IContextKeyService,
		@IViewDescriptorService viewDescriptorService: IViewDescriptorService,
		@IInstantiationService instantiationService: IInstantiationService,
		@IOpenerService openerService: IOpenerService,
		@IThemeService themeService: IThemeService,
		@ITelemetryService telemetryService: ITelemetryService,
		@IHoverService hoverService: IHoverService,
	) {
		super(options, keybindingService, contextMenuService, configurationService, contextKeyService, viewDescriptorService, instantiationService, openerService, themeService, hoverService);
	}

	protected override renderBody(container: HTMLElement): void {
		super.renderBody(container);
		container.classList.add('forge-mcp-status-view');

		this.listContainer = document.createElement('div');
		this.listContainer.className = 'forge-mcp-server-list';
		container.appendChild(this.listContainer);

		this._register(this.forgeMcpService.onDidChangeServerStatus(() => {
			this.renderServerList();
		}));

		this._register(this.forgeMcpService.onDidChangeTools(() => {
			this.renderServerList();
		}));

		this.renderServerList();
	}

	private renderServerList(): void {
		this.listContainer.innerHTML = '';
		const statuses = this.forgeMcpService.getServerStatuses();

		if (statuses.length === 0) {
			const empty = document.createElement('div');
			empty.className = 'forge-mcp-empty';
			empty.textContent = 'No MCP servers configured';
			this.listContainer.appendChild(empty);
			return;
		}

		for (const server of statuses) {
			this.listContainer.appendChild(this.renderServerRow(server));
		}
	}

	private renderServerRow(server: ForgeMcpServerStatusEntry): HTMLElement {
		const row = document.createElement('div');
		row.className = 'forge-mcp-server-row';
		if (server.disabled) {
			row.classList.add('disabled');
		}

		const dot = document.createElement('span');
		dot.className = 'forge-mcp-server-dot';
		if (server.disabled) {
			dot.classList.add('disabled');
		} else {
			switch (server.status) {
				case ForgeMcpServerStatus.Connected:
					dot.classList.add('connected');
					break;
				case ForgeMcpServerStatus.Connecting:
					dot.classList.add('connecting');
					break;
				case ForgeMcpServerStatus.Error:
					dot.classList.add('error');
					break;
				default:
					dot.classList.add('disconnected');
			}
		}
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
		toolCount.textContent = server.disabled ? 'disabled' : `${server.toolCount} tools`;
		row.appendChild(toolCount);

		const toggle = document.createElement('button');
		toggle.className = 'forge-mcp-server-toggle';
		toggle.title = server.disabled ? `Enable ${server.name}` : `Disable ${server.name}`;
		toggle.textContent = server.disabled ? 'Enable' : 'Disable';
		toggle.addEventListener('click', (e) => {
			e.stopPropagation();
			this.forgeMcpService.toggleServerDisabled(server.name, !server.disabled);
		});
		row.appendChild(toggle);

		return row;
	}

	protected override layoutBody(height: number, width: number): void {
		super.layoutBody(height, width);
	}
}
