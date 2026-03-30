/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { KeyCode, KeyMod } from '../../../../base/common/keyCodes.js';
import * as nls from '../../../../nls.js';
import { SyncDescriptor } from '../../../../platform/instantiation/common/descriptors.js';
import { Registry } from '../../../../platform/registry/common/platform.js';
import { Extensions as ViewExtensions, IViewContainersRegistry, IViewDescriptor, IViewsRegistry, ViewContainerLocation } from '../../../common/views.js';
import { FORGE_AI_VIEWLET_ID, FORGE_AI_WORKSPACE_VIEW_ID } from '../common/forgeAI.js';
import { ForgeAIViewPaneContainer, forgeAIViewIcon } from './forgeAIViewlet.js';
import { ForgeAIWorkspaceView } from './forgeAIWorkspaceView.js';

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
	name: nls.localize2('forgeAI.workspaces', "AI Workspaces"),
	ctorDescriptor: new SyncDescriptor(ForgeAIWorkspaceView),
	canToggleVisibility: false,
	canMoveView: true,
};

Registry.as<IViewsRegistry>(ViewExtensions.ViewsRegistry).registerViews([viewDescriptor], viewContainer);
