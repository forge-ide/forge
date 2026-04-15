/*---------------------------------------------------------------------------------------------
 * Forge - ForgeChatsView (tabbed: Chats + Providers)
 *--------------------------------------------------------------------------------------------*/

import { addDisposableListener, EventType, reset } from '../../../../base/browser/dom.js';
import { localize } from '../../../../nls.js';
import { URI } from '../../../../base/common/uri.js';
import { IKeybindingService } from '../../../../platform/keybinding/common/keybinding.js';
import { IContextMenuService } from '../../../../platform/contextview/browser/contextView.js';
import { IConfigurationService } from '../../../../platform/configuration/common/configuration.js';
import { IContextKeyService } from '../../../../platform/contextkey/common/contextkey.js';
import { IInstantiationService } from '../../../../platform/instantiation/common/instantiation.js';
import { IOpenerService } from '../../../../platform/opener/common/opener.js';
import { IThemeService } from '../../../../platform/theme/common/themeService.js';
import { IHoverService } from '../../../../platform/hover/browser/hover.js';
import { IDialogService } from '../../../../platform/dialogs/common/dialogs.js';
import { ILogService } from '../../../../platform/log/common/log.js';
import { ViewPane } from '../../../browser/parts/views/viewPane.js';
import { IViewletViewOptions } from '../../../browser/parts/views/viewsViewlet.js';
import { IViewDescriptorService } from '../../../common/views.js';
import { IForgeChatService, ForgeChatEntry } from '../../../services/forge/common/forgeChatService.js';
import { ISessionsManagementService } from '../../../../sessions/contrib/sessions/browser/sessionsManagementService.js';
import { IForgeConfigService } from '../../../services/forge/common/forgeConfigService.js';
import { IAIProviderService } from '../../../../platform/ai/common/aiProviderService.js';
import {
	createProviderHeader,
	createChatRow,
	createExpandedChatRow,
	createProviderCard,
} from './forgeChatsViewHelpers.js';

export const FORGE_CHATS_VIEW_ID = 'workbench.forgeChats.mainView';

type TabName = 'chats' | 'providers';

export class ForgeChatsView extends ViewPane {

	private _tabBar!: HTMLElement;
	private _tabChats!: HTMLElement;
	private _tabProviders!: HTMLElement;
	private _paneChats!: HTMLElement;
	private _paneProviders!: HTMLElement;
	private _activeTab: TabName = 'chats';
	private _expandedResource: string | undefined;

	constructor(
		options: IViewletViewOptions,
		@IForgeChatService private readonly _chatService: IForgeChatService,
		@ISessionsManagementService private readonly _sessionsMgmtService: ISessionsManagementService,
		@IForgeConfigService private readonly _configService: IForgeConfigService,
		@IAIProviderService private readonly _aiProviderService: IAIProviderService,
		@IDialogService private readonly _dialogService: IDialogService,
		@ILogService private readonly _logService: ILogService,
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
		container.classList.add('forge-chats-view');
		// Tab bar
		this._tabBar = document.createElement('div');
		this._tabBar.className = 'forge-chats-tab-bar';
		container.appendChild(this._tabBar);

		this._tabChats = document.createElement('div');
		this._tabChats.className = 'forge-chats-tab';
		this._tabChats.textContent = localize('forgeChats.tab.chats', 'Chats');
		this._register(addDisposableListener(this._tabChats, EventType.CLICK, () => this._switchTab('chats')));
		this._tabBar.appendChild(this._tabChats);

		this._tabProviders = document.createElement('div');
		this._tabProviders.className = 'forge-chats-tab';
		this._tabProviders.textContent = localize('forgeChats.tab.providers', 'Providers');
		this._register(addDisposableListener(this._tabProviders, EventType.CLICK, () => this._switchTab('providers')));
		this._tabBar.appendChild(this._tabProviders);

		// Panes
		this._paneChats = document.createElement('div');
		this._paneChats.className = 'forge-chats-pane';
		container.appendChild(this._paneChats);

		this._paneProviders = document.createElement('div');
		this._paneProviders.className = 'forge-chats-pane forge-chats-pane--hidden';
		container.appendChild(this._paneProviders);

		// Subscribe to changes
		this._register(this._chatService.onDidChangeChats(() => {
			if (this._activeTab === 'chats') {
				this._renderChats();
			}
		}));

		this._register(this._configService.onDidChange(() => {
			if (this._activeTab === 'providers') {
				this._renderProviders();
			}
		}));

		this._register(this._aiProviderService.onDidChangeProviders(() => {
			if (this._activeTab === 'providers') {
				this._renderProviders();
			}
		}));

		this._switchTab('chats');
	}

	private _switchTab(tab: TabName): void {
		this._activeTab = tab;

		this._tabChats.classList.toggle('forge-chats-tab--active', tab === 'chats');
		this._tabProviders.classList.toggle('forge-chats-tab--active', tab === 'providers');

		if (tab === 'chats') {
			this._paneChats.classList.remove('forge-chats-pane--hidden');
			this._paneProviders.classList.add('forge-chats-pane--hidden');
			this._renderChats();
		} else {
			this._paneProviders.classList.remove('forge-chats-pane--hidden');
			this._paneChats.classList.add('forge-chats-pane--hidden');
			this._renderProviders();
		}
	}

	private _renderChats(): void {
		reset(this._paneChats);

		const chats = this._chatService.getChats();

		if (chats.length === 0) {
			const empty = document.createElement('div');
			empty.className = 'forge-chats-empty';
			empty.textContent = localize('forgeChats.empty', 'No chats yet. Start one from the Providers tab.');
			this._paneChats.appendChild(empty);
			return;
		}

		// Group by providerName
		const byProvider = new Map<string, ForgeChatEntry[]>();
		for (const chat of chats) {
			const list = byProvider.get(chat.providerName) ?? [];
			list.push(chat);
			byProvider.set(chat.providerName, list);
		}

		for (const [providerName, providerChats] of byProvider) {
			this._paneChats.appendChild(createProviderHeader(providerName));

			for (const chat of providerChats) {
				const resourceKey = chat.resource.toString();
				const isExpanded = this._expandedResource === resourceKey;

				if (isExpanded) {
					const expandedRow = createExpandedChatRow(
						chat,
						() => this._openChat(chat.resource),
						() => { this._renameChat(chat).catch(err => this._logService.error('ForgeChatsView: rename failed', err)); },
						() => { this._deleteChat(chat.resource).catch(err => this._logService.error('ForgeChatsView: delete failed', err)); },
					);
					this._register(addDisposableListener(expandedRow, EventType.CLICK, () => this._toggleExpand(resourceKey)));
					this._paneChats.appendChild(expandedRow);
				} else {
					const row = createChatRow(chat);
					this._register(addDisposableListener(row, EventType.CLICK, () => this._toggleExpand(resourceKey)));
					this._paneChats.appendChild(row);
				}
			}
		}
	}

	private _toggleExpand(resourceKey: string): void {
		this._expandedResource = this._expandedResource === resourceKey ? undefined : resourceKey;
		this._renderChats();
	}

	private _renderProviders(): void {
		reset(this._paneProviders);

		const providers = this._configService.getProviders();

		if (providers.length === 0) {
			const empty = document.createElement('div');
			empty.className = 'forge-chats-empty';
			empty.textContent = localize('forgeChats.noProviders', 'No providers configured.');
			this._paneProviders.appendChild(empty);
			return;
		}

		for (const provider of providers) {
			const isConfigured = this._aiProviderService.has(provider.name);
			const card = createProviderCard(
				provider,
				(_providerName, _modelId) => { this._chatService.openNewChat(); this._switchTab('chats'); },
				isConfigured,
			);
			this._paneProviders.appendChild(card);
		}
	}

	private _openChat(resource: URI): void {
		this._sessionsMgmtService.openSession(resource).catch((err: unknown) => this._logService.error('ForgeChatsView: openSession failed', err));
	}

	private async _renameChat(chat: ForgeChatEntry): Promise<void> {
		const result = await this._dialogService.input({
			message: localize('forgeChats.rename.message', 'Rename Chat'),
			inputs: [{ type: 'text', value: chat.label, placeholder: localize('forgeChats.rename.placeholder', 'Chat name') }],
			primaryButton: localize('forgeChats.rename.confirm', 'Rename'),
		});

		if (result.confirmed && result.values?.[0]) {
			this._chatService.renameChat(chat.resource, result.values[0]);
		}
	}

	private async _deleteChat(resource: URI): Promise<void> {
		const result = await this._dialogService.confirm({
			message: localize('forgeChats.delete.message', 'Delete this chat?'),
			detail: localize('forgeChats.delete.detail', 'This action cannot be undone.'),
			primaryButton: localize('forgeChats.delete.confirm', 'Delete'),
		});

		if (result.confirmed) {
			if (this._expandedResource === resource.toString()) {
				this._expandedResource = undefined;
			}
			this._chatService.deleteChat(resource);
		}
	}

	protected override layoutBody(height: number, width: number): void {
		super.layoutBody(height, width);
	}

	override getOptimalWidth(): number {
		return 300;
	}
}
