/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { $, addDisposableListener, append, clearNode } from '../../../../base/browser/dom.js';
import { DisposableStore } from '../../../../base/common/lifecycle.js';
import { localize } from '../../../../nls.js';
import { toAction } from '../../../../base/common/actions.js';
import { ICommandService } from '../../../../platform/commands/common/commands.js';
import { IConfigurationService } from '../../../../platform/configuration/common/configuration.js';
import { IContextKeyService, IContextKey } from '../../../../platform/contextkey/common/contextkey.js';
import { IContextMenuService } from '../../../../platform/contextview/browser/contextView.js';
import { IHoverService } from '../../../../platform/hover/browser/hover.js';
import { IInstantiationService } from '../../../../platform/instantiation/common/instantiation.js';
import { IKeybindingService } from '../../../../platform/keybinding/common/keybinding.js';
import { IOpenerService } from '../../../../platform/opener/common/opener.js';
import { IQuickInputService } from '../../../../platform/quickinput/common/quickInput.js';
import { IThemeService } from '../../../../platform/theme/common/themeService.js';
import { IViewDescriptorService } from '../../../common/views.js';
import { IViewletViewOptions } from '../../../browser/parts/views/viewsViewlet.js';
import { ViewPane } from '../../../browser/parts/views/viewPane.js';
import { IForgeConfigService } from '../../../services/forge/common/forgeConfigService.js';
import { ForgeLayout, IForgeLayoutService } from '../../../services/forge/common/forgeLayoutService.js';
import { IForgeWorkspaceService } from '../../../services/forge/common/forgeWorkspaceService.js';
import type { ForgeWorkspaceConfig } from '../../../services/forge/common/forgeWorkspaceTypes.js';
import { FORGE_AI_HAS_WORKSPACES } from '../common/forgeAI.js';
import './media/forgeLayoutButtons.css';

interface LayoutButtonConfig {
	readonly layout: ForgeLayout;
	readonly command: string;
	readonly title: string;
	readonly svgPath: string;
}

const LAYOUT_BUTTONS: readonly LayoutButtonConfig[] = [
	{ layout: 'focus', command: 'forge.layout.focus', title: 'Focus Mode', svgPath: 'M3 3h18v18H3z' },
	{ layout: 'split', command: 'forge.layout.split', title: 'Split Canvas', svgPath: 'M3 3h8v18H3zM13 3h8v18h-8z' },
	{ layout: 'code+ai', command: 'forge.layout.codeai', title: 'Code + AI', svgPath: 'M3 3h12v18H3zM17 3h4v18h-4z' },
	{ layout: 'quad', command: 'forge.layout.quad', title: 'Quad Canvas', svgPath: 'M3 3h8v8H3zM13 3h8v8h-8zM3 13h8v8H3zM13 13h8v8h-8z' },
];

const SVG_NS = 'http://www.w3.org/2000/svg';

function createSvgIcon(pathData: string): SVGElement {
	const svg = document.createElementNS(SVG_NS, 'svg');
	svg.setAttribute('viewBox', '0 0 24 24');
	const path = document.createElementNS(SVG_NS, 'path');
	path.setAttribute('d', pathData);
	svg.appendChild(path);
	return svg;
}

const LAYOUT_ICONS: Record<ForgeLayout, string> = Object.fromEntries(
	LAYOUT_BUTTONS.map(b => [b.layout, b.svgPath])
) as Record<ForgeLayout, string>;

export class ForgeAIWorkspaceView extends ViewPane {

	static readonly ID = 'workbench.forgeAI.workspaceView';

	private providerLabel: HTMLElement | undefined;
	private readonly layoutButtons = new Map<ForgeLayout, HTMLButtonElement>();
	private workspaceListContainer: HTMLElement | undefined;
	private readonly hasWorkspacesKey: IContextKey<boolean>;
	private readonly _workspaceListListenerStore = this._register(new DisposableStore());

	constructor(
		options: IViewletViewOptions,
		@ICommandService private readonly commandService: ICommandService,
		@IForgeConfigService private readonly forgeConfigService: IForgeConfigService,
		@IForgeLayoutService private readonly forgeLayoutService: IForgeLayoutService,
		@IForgeWorkspaceService private readonly forgeWorkspaceService: IForgeWorkspaceService,
		@IQuickInputService private readonly quickInputService: IQuickInputService,
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

		this.hasWorkspacesKey = FORGE_AI_HAS_WORKSPACES.bindTo(contextKeyService);

		this._register(this.forgeConfigService.onDidChange(() => this.updateProviderDisplay()));
		this._register(this.forgeLayoutService.onDidChangeLayout(layout => this.updateActiveLayoutButton(layout)));
		this._register(this.forgeWorkspaceService.onDidChangeWorkspaces(() => this.renderWorkspaceList()));
		this._register(this.forgeWorkspaceService.onDidChangeActiveWorkspace(() => this.renderWorkspaceList()));
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

		// Canvas layout section
		const layoutSection = append(body, $('.forge-layout-section'));
		const layoutHeading = append(layoutSection, $('.forge-ai-section-heading'));
		layoutHeading.textContent = localize('forgeAI.canvasLayout', "CANVAS LAYOUT");

		const buttonContainer = append(layoutSection, $('.forge-layout-buttons'));

		for (const config of LAYOUT_BUTTONS) {
			const btn = append(buttonContainer, $('button.forge-layout-btn')) as HTMLButtonElement;
			btn.type = 'button';
			btn.title = config.title;
			btn.dataset['layout'] = config.layout;
			btn.appendChild(createSvgIcon(config.svgPath));

			this.layoutButtons.set(config.layout, btn);

			this._register(addDisposableListener(btn, 'click', () => {
				this.commandService.executeCommand(config.command);
			}));
		}

		// Set initial active state
		this.updateActiveLayoutButton(this.forgeLayoutService.activeLayout);

		// Workspace section
		const workspaceSection = append(body, $('.forge-workspace-section'));
		const workspaceHeading = append(workspaceSection, $('.forge-ai-section-heading'));
		workspaceHeading.textContent = localize('forgeAI.workspaces', "AGENTIC WORKSPACES");

		// Workspace action buttons
		const workspaceActions = append(workspaceSection, $('.forge-workspace-actions'));

		const saveBtn = append(workspaceActions, $('button.forge-workspace-action-btn')) as HTMLButtonElement;
		saveBtn.type = 'button';
		saveBtn.textContent = localize('forgeAI.saveWorkspace', "SAVE");
		saveBtn.title = localize('forgeAI.saveWorkspaceTitle', "Save current agentic workspace");
		this._register(addDisposableListener(saveBtn, 'click', () => this.handleSaveWorkspace()));

		const newBtn = append(workspaceActions, $('button.forge-workspace-action-btn')) as HTMLButtonElement;
		newBtn.type = 'button';
		newBtn.textContent = localize('forgeAI.newWorkspace', "NEW");
		newBtn.title = localize('forgeAI.newWorkspaceTitle', "Create new agentic workspace");
		this._register(addDisposableListener(newBtn, 'click', () => this.handleNewWorkspace()));

		// Workspace list
		this.workspaceListContainer = append(workspaceSection, $('.forge-workspace-list'));
		this.renderWorkspaceList();
	}

	private readonly handleNewChat = (): void => {
		this.commandService.executeCommand('forge.chat.new');
	};

	private async handleSaveWorkspace(): Promise<void> {
		await this.commandService.executeCommand('forge.workspace.save');
	}

	private async handleNewWorkspace(): Promise<void> {
		await this.commandService.executeCommand('forge.workspace.create');
	}

	private renderWorkspaceList(): void {
		if (!this.workspaceListContainer) {
			return;
		}

		this._workspaceListListenerStore.clear();
		clearNode(this.workspaceListContainer);

		const workspaces = this.forgeWorkspaceService.getWorkspaces();
		const activeWorkspace = this.forgeWorkspaceService.getActiveWorkspace();

		this.hasWorkspacesKey.set(workspaces.length > 0);

		if (workspaces.length === 0) {
			const emptyLabel = append(this.workspaceListContainer, $('.forge-workspace-empty'));
			emptyLabel.textContent = localize('forgeAI.noWorkspaces', "No saved agentic workspaces");
			return;
		}

		for (const workspace of workspaces) {
			const isActive = activeWorkspace?.id === workspace.id;
			this.renderWorkspaceItem(workspace, isActive);
		}
	}

	private renderWorkspaceItem(workspace: ForgeWorkspaceConfig, isActive: boolean): void {
		if (!this.workspaceListContainer) {
			return;
		}

		const item = append(this.workspaceListContainer, $('.forge-workspace-item'));
		if (isActive) {
			item.classList.add('active');
		}

		// Layout icon
		const iconSvgPath = LAYOUT_ICONS[workspace.layout] ?? LAYOUT_ICONS['focus'];
		const icon = append(item, $('.forge-workspace-item-icon'));
		icon.appendChild(createSvgIcon(iconSvgPath));

		// Name
		const name = append(item, $('.forge-workspace-item-name'));
		name.textContent = workspace.name;

		// Click to switch
		this._workspaceListListenerStore.add(addDisposableListener(item, 'click', () => {
			this.forgeWorkspaceService.switchWorkspace(workspace.id);
		}));

		// Right-click context menu
		this._workspaceListListenerStore.add(addDisposableListener(item, 'contextmenu', (e: MouseEvent) => {
			e.preventDefault();
			e.stopPropagation();
			this.showWorkspaceContextMenu(workspace, e);
		}));
	}

	private showWorkspaceContextMenu(workspace: ForgeWorkspaceConfig, event: MouseEvent): void {
		const actions = [
			toAction({
				id: 'forge.workspace.rename',
				label: localize('forgeAI.renameWorkspace', "Rename"),
				run: async () => {
					const newName = await this.quickInputService.input({
						placeHolder: localize('forgeAI.workspaceName', "Agentic workspace name"),
						prompt: localize('forgeAI.renameWorkspacePrompt', "Enter a new name for the agentic workspace"),
						value: workspace.name,
					});
					if (newName && newName !== workspace.name) {
						await this.forgeWorkspaceService.renameWorkspace(workspace.id, newName);
					}
				},
			}),
			toAction({
				id: 'forge.workspace.delete',
				label: localize('forgeAI.deleteWorkspace', "Delete"),
				run: async () => {
					await this.forgeWorkspaceService.deleteWorkspace(workspace.id);
				},
			}),
		];

		this.contextMenuService.showContextMenu({
			getAnchor: () => ({ x: event.clientX, y: event.clientY }),
			getActions: () => actions,
		});
	}

	private updateProviderDisplay(): void {
		if (!this.providerLabel) {
			return;
		}

		const config = this.forgeConfigService.getConfig();
		const providerName = config.provider || localize('forgeAI.noProvider', "None");
		const modelName = config.model ? ` / ${config.model}` : '';
		this.providerLabel.textContent = providerName + modelName;
	}

	private updateActiveLayoutButton(activeLayout: ForgeLayout): void {
		for (const [layout, btn] of this.layoutButtons) {
			btn.classList.toggle('active', layout === activeLayout);
		}
	}
}
