/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { KeyCode, KeyMod } from '../../../../base/common/keyCodes.js';
import * as nls from '../../../../nls.js';
import { Action2, registerAction2 } from '../../../../platform/actions/common/actions.js';
import { SyncDescriptor } from '../../../../platform/instantiation/common/descriptors.js';
import { registerSingleton, InstantiationType } from '../../../../platform/instantiation/common/extensions.js';
import { ServicesAccessor } from '../../../../platform/instantiation/common/instantiation.js';
import { IQuickInputService, IQuickPickItem } from '../../../../platform/quickinput/common/quickInput.js';
import { Registry } from '../../../../platform/registry/common/platform.js';
import { Extensions as ViewExtensions, IViewContainersRegistry, IViewDescriptor, IViewsRegistry, ViewContainerLocation } from '../../../common/views.js';
import { IForgeWorkspaceService } from '../../../services/forge/common/forgeWorkspaceService.js';
import { IForgeMcpBridgeHost } from '../../../services/forge/browser/forgeMcpService.js';
import { FORGE_AI_VIEWLET_ID, FORGE_AI_WORKSPACE_VIEW_ID } from '../common/forgeAI.js';
import { ForgeAIViewPaneContainer, forgeAIViewIcon } from './forgeAIViewlet.js';
import { ForgeAIWorkspaceView } from './forgeAIWorkspaceView.js';
import { ForgeMcpStatusView, FORGE_MCP_STATUS_VIEW_ID } from './forgeMcpStatusView.js';
import { ForgeAgentMonitorView, FORGE_AGENT_MONITOR_VIEW_ID } from './forgeAgentMonitorView.js';
import { ForgeMcpBridgeHost } from './forgeMcpBridgeHost.js';

// --- Service Registrations ---

// Bridge between VS Code's IMcpService (contrib/mcp) and ForgeMcpService (services/forge).
// Registered here because this is the one contrib file that can see both.
registerSingleton(IForgeMcpBridgeHost, ForgeMcpBridgeHost, InstantiationType.Delayed);

// --- View Container Registration ---

const viewContainer = Registry.as<IViewContainersRegistry>(ViewExtensions.ViewContainersRegistry).registerViewContainer({
	id: FORGE_AI_VIEWLET_ID,
	title: nls.localize2('forgeAI', "Forge AI"),
	ctorDescriptor: new SyncDescriptor(ForgeAIViewPaneContainer),
	icon: forgeAIViewIcon,
	order: 10,
	openCommandActionDescriptor: {
		id: FORGE_AI_VIEWLET_ID,
		mnemonicTitle: nls.localize({ key: 'miViewForgeAI', comment: ['&& denotes a mnemonic'] }, "Forge &&AI"),
		keybindings: {
			primary: KeyMod.CtrlCmd | KeyMod.Alt | KeyCode.KeyA,
		},
		order: 10,
	},
}, ViewContainerLocation.Sidebar);

// --- View Registration ---

const viewDescriptor: IViewDescriptor = {
	id: FORGE_AI_WORKSPACE_VIEW_ID,
	containerIcon: forgeAIViewIcon,
	name: nls.localize2('forgeAI.workspaces', "Agentic Workspaces"),
	ctorDescriptor: new SyncDescriptor(ForgeAIWorkspaceView),
	canToggleVisibility: false,
	canMoveView: true,
};

const mcpStatusViewDescriptor: IViewDescriptor = {
	id: FORGE_MCP_STATUS_VIEW_ID,
	containerIcon: forgeAIViewIcon,
	name: nls.localize2('forgeAI.mcpServers', "MCP Servers"),
	ctorDescriptor: new SyncDescriptor(ForgeMcpStatusView),
	order: 2,
	canToggleVisibility: true,
	canMoveView: false,
	weight: 30,
};

const agentMonitorViewDescriptor: IViewDescriptor = {
	id: FORGE_AGENT_MONITOR_VIEW_ID,
	containerIcon: forgeAIViewIcon,
	name: nls.localize2('forgeAI.agents', "Agents"),
	ctorDescriptor: new SyncDescriptor(ForgeAgentMonitorView),
	order: 3,
	canToggleVisibility: true,
	canMoveView: false,
	weight: 30,
};

Registry.as<IViewsRegistry>(ViewExtensions.ViewsRegistry).registerViews([viewDescriptor, mcpStatusViewDescriptor, agentMonitorViewDescriptor], viewContainer);

// --- Workspace Commands ---

registerAction2(class extends Action2 {
	constructor() {
		super({
			id: 'forge.workspace.create',
			title: nls.localize2('forgeWorkspace.create', "Forge: Create Agentic Workspace"),
			f1: true,
		});
	}

	async run(accessor: ServicesAccessor): Promise<void> {
		const quickInputService = accessor.get(IQuickInputService);
		const workspaceService = accessor.get(IForgeWorkspaceService);

		const name = await quickInputService.input({
			placeHolder: nls.localize('forgeWorkspace.namePlaceholder', "Agentic workspace name"),
			prompt: nls.localize('forgeWorkspace.namePrompt', "Enter a name for the new agentic workspace"),
		});

		if (name) {
			await workspaceService.createWorkspace(name);
		}
	}
});

registerAction2(class extends Action2 {
	constructor() {
		super({
			id: 'forge.workspace.save',
			title: nls.localize2('forgeWorkspace.save', "Forge: Save Agentic Workspace"),
			f1: true,
		});
	}

	async run(accessor: ServicesAccessor): Promise<void> {
		const workspaceService = accessor.get(IForgeWorkspaceService);
		const quickInputService = accessor.get(IQuickInputService);

		const active = workspaceService.getActiveWorkspace();
		if (active) {
			await workspaceService.saveActiveWorkspace();
		} else {
			const name = await quickInputService.input({
				placeHolder: nls.localize('forgeWorkspace.namePlaceholder', "Agentic workspace name"),
				prompt: nls.localize('forgeWorkspace.namePrompt', "Enter a name for the new agentic workspace"),
			});
			if (name) {
				await workspaceService.createWorkspace(name);
			}
		}
	}
});

registerAction2(class extends Action2 {
	constructor() {
		super({
			id: 'forge.workspace.switch',
			title: nls.localize2('forgeWorkspace.switch', "Forge: Switch Agentic Workspace"),
			f1: true,
		});
	}

	async run(accessor: ServicesAccessor): Promise<void> {
		const quickInputService = accessor.get(IQuickInputService);
		const workspaceService = accessor.get(IForgeWorkspaceService);

		const workspaces = workspaceService.getWorkspaces();
		if (workspaces.length === 0) {
			return;
		}

		const activeWorkspace = workspaceService.getActiveWorkspace();

		const items: IQuickPickItem[] = workspaces.map(w => ({
			label: w.name,
			description: `${w.layout} layout`,
			detail: activeWorkspace?.id === w.id ? nls.localize('forgeWorkspace.active', "(active)") : undefined,
			id: w.id,
		}));

		const picked = await quickInputService.pick(items, {
			placeHolder: nls.localize('forgeWorkspace.switchPlaceholder', "Select an agentic workspace to switch to"),
		});

		if (picked && picked.id) {
			await workspaceService.switchWorkspace(picked.id);
		}
	}
});

registerAction2(class extends Action2 {
	constructor() {
		super({
			id: 'forge.workspace.delete',
			title: nls.localize2('forgeWorkspace.delete', "Forge: Delete Agentic Workspace"),
			f1: true,
		});
	}

	async run(accessor: ServicesAccessor): Promise<void> {
		const quickInputService = accessor.get(IQuickInputService);
		const workspaceService = accessor.get(IForgeWorkspaceService);

		const workspaces = workspaceService.getWorkspaces();
		if (workspaces.length === 0) {
			return;
		}

		const items: IQuickPickItem[] = workspaces.map(w => ({
			label: w.name,
			description: `${w.layout} layout`,
			id: w.id,
		}));

		const picked = await quickInputService.pick(items, {
			placeHolder: nls.localize('forgeWorkspace.deletePlaceholder', "Select an agentic workspace to delete"),
		});

		if (picked && picked.id) {
			await workspaceService.deleteWorkspace(picked.id);
		}
	}
});
