/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import * as nls from '../../../../nls.js';
import { Action2, registerAction2 } from '../../../../platform/actions/common/actions.js';
import { ServicesAccessor } from '../../../../platform/instantiation/common/instantiation.js';
import { IQuickInputService, IQuickPickItem } from '../../../../platform/quickinput/common/quickInput.js';
import { IForgeWorkspaceService } from '../../../services/forge/common/forgeWorkspaceService.js';

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
