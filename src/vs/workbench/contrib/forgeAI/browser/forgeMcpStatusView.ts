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
import { reset } from '../../../../base/browser/dom.js';
import { ViewPane } from '../../../browser/parts/views/viewPane.js';
import { IViewletViewOptions } from '../../../browser/parts/views/viewsViewlet.js';
import { IForgeMcpService, ForgeMcpServerStatusEntry } from '../../../services/forge/common/forgeMcpService.js';
import { createEmptyServersState, createServerRow } from './forgeMcpStatusViewHelpers.js';

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
		reset(this.listContainer);
		const statuses = this.forgeMcpService.getServerStatuses();

		if (statuses.length === 0) {
			this.listContainer.appendChild(createEmptyServersState());
			return;
		}

		for (const server of statuses) {
			this.listContainer.appendChild(this.renderServerRow(server));
		}
	}

	private renderServerRow(server: ForgeMcpServerStatusEntry): HTMLElement {
		return createServerRow(server, () => this.forgeMcpService.toggleServerDisabled(server.name, !server.disabled));
	}

	protected override layoutBody(height: number, width: number): void {
		super.layoutBody(height, width);
	}
}
