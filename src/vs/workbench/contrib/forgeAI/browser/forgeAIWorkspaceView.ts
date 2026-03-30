/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { $, addDisposableListener, append } from '../../../../base/browser/dom.js';
import { localize } from '../../../../nls.js';
import { ICommandService } from '../../../../platform/commands/common/commands.js';
import { IConfigurationService } from '../../../../platform/configuration/common/configuration.js';
import { IContextKeyService } from '../../../../platform/contextkey/common/contextkey.js';
import { IContextMenuService } from '../../../../platform/contextview/browser/contextView.js';
import { IHoverService } from '../../../../platform/hover/browser/hover.js';
import { IInstantiationService } from '../../../../platform/instantiation/common/instantiation.js';
import { IKeybindingService } from '../../../../platform/keybinding/common/keybinding.js';
import { IOpenerService } from '../../../../platform/opener/common/opener.js';
import { IThemeService } from '../../../../platform/theme/common/themeService.js';
import { IViewDescriptorService } from '../../../common/views.js';
import { IViewletViewOptions } from '../../../browser/parts/views/viewsViewlet.js';
import { ViewPane } from '../../../browser/parts/views/viewPane.js';
import { IForgeConfigService } from '../../../services/forge/common/forgeConfigService.js';

export class ForgeAIWorkspaceView extends ViewPane {

	static readonly ID = 'workbench.forgeAI.workspaceView';

	private providerLabel: HTMLElement | undefined;

	constructor(
		options: IViewletViewOptions,
		@ICommandService private readonly commandService: ICommandService,
		@IForgeConfigService private readonly forgeConfigService: IForgeConfigService,
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

		this._register(this.forgeConfigService.onDidChange(() => this.updateProviderDisplay()));
	}

	protected override renderBody(container: HTMLElement): void {
		super.renderBody(container);

		const body = append(container, $('.forge-ai-workspace-body'));

		// Provider info section
		const providerSection = append(body, $('.forge-ai-provider-section'));
		const providerHeading = append(providerSection, $('.forge-ai-section-heading'));
		providerHeading.textContent = localize('forgeAI.activeProvider', "ACTIVE PROVIDER");

		this.providerLabel = append(providerSection, $('.forge-ai-provider-label'));
		this.updateProviderDisplay();

		// New Chat button
		const newChatButton = append(body, $('button.forge-ai-new-chat-button')) as HTMLButtonElement;
		newChatButton.textContent = localize('forgeAI.newChat', "NEW CHAT");
		newChatButton.type = 'button';

		this._register(addDisposableListener(newChatButton, 'click', () => this.handleNewChat()));
	}

	private readonly handleNewChat = (): void => {
		this.commandService.executeCommand('forge.chat.new');
	};

	private updateProviderDisplay(): void {
		if (!this.providerLabel) {
			return;
		}

		const config = this.forgeConfigService.getConfig();
		const providerName = config.provider || localize('forgeAI.noProvider', "None");
		const modelName = config.model ? ` / ${config.model}` : '';
		this.providerLabel.textContent = providerName + modelName;
	}
}
