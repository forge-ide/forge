/*---------------------------------------------------------------------------------------------
 * Forge - Agents activity bar tab
 *--------------------------------------------------------------------------------------------*/

import * as nls from '../../../../nls.js';
import { Registry } from '../../../../platform/registry/common/platform.js';
import { SyncDescriptor } from '../../../../platform/instantiation/common/descriptors.js';
import { registerSingleton, InstantiationType } from '../../../../platform/instantiation/common/extensions.js';
import { Extensions as ViewExtensions, IViewContainersRegistry, IViewDescriptor, IViewsRegistry, ViewContainerLocation } from '../../../common/views.js';
import { ForgeAgentsViewPaneContainer, forgeAgentsViewIcon, FORGE_AGENTS_VIEWLET_ID } from './forgeAgentsViewlet.js';
import { ForgeAgentsView, FORGE_AGENTS_VIEW_ID } from './forgeAgentsView.js';
import { IForgeSessionContextService } from '../../../services/forge/common/forgeSessionContextService.js';
import { ForgeSessionContextService } from '../../../services/forge/browser/forgeSessionContextService.js';
import './media/forgeAgents.css';

registerSingleton(IForgeSessionContextService, ForgeSessionContextService, InstantiationType.Delayed);

const viewContainer = Registry.as<IViewContainersRegistry>(ViewExtensions.ViewContainersRegistry).registerViewContainer({
	id: FORGE_AGENTS_VIEWLET_ID,
	title: nls.localize2('forgeAgents', 'Agents'),
	ctorDescriptor: new SyncDescriptor(ForgeAgentsViewPaneContainer),
	icon: forgeAgentsViewIcon,
	order: 11,
	openCommandActionDescriptor: {
		id: FORGE_AGENTS_VIEWLET_ID,
		mnemonicTitle: nls.localize({ key: 'miViewForgeAgents', comment: ['&& denotes a mnemonic'] }, '&&Agents'),
		order: 11,
	},
}, ViewContainerLocation.Sidebar);

const agentsViewDescriptor: IViewDescriptor = {
	id: FORGE_AGENTS_VIEW_ID,
	containerIcon: forgeAgentsViewIcon,
	name: nls.localize2('forgeAgents.view', 'Agents'),
	ctorDescriptor: new SyncDescriptor(ForgeAgentsView),
	order: 1,
	canToggleVisibility: false,
	canMoveView: false,
};

Registry.as<IViewsRegistry>(ViewExtensions.ViewsRegistry).registerViews([agentsViewDescriptor], viewContainer);
