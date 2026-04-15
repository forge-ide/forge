/*---------------------------------------------------------------------------------------------
 * Forge - Chats activity bar tab
 *--------------------------------------------------------------------------------------------*/

import * as nls from '../../../../nls.js';
import { Registry } from '../../../../platform/registry/common/platform.js';
import { SyncDescriptor } from '../../../../platform/instantiation/common/descriptors.js';
import { registerSingleton, InstantiationType } from '../../../../platform/instantiation/common/extensions.js';
import { Extensions as ViewExtensions, IViewContainersRegistry, IViewDescriptor, IViewsRegistry, ViewContainerLocation } from '../../../common/views.js';
import { ForgeChatsViewPaneContainer, forgeChatsViewIcon, FORGE_CHATS_VIEWLET_ID } from './forgeChatsViewlet.js';
import { ForgeChatsView, FORGE_CHATS_VIEW_ID } from './forgeChatsView.js';
import { IForgeChatService } from '../../../services/forge/common/forgeChatService.js';
import { ForgeChatService } from '../../../services/forge/browser/forgeChatService.js';
import { ISessionsManagementService, SessionsManagementService } from '../../../../sessions/contrib/sessions/browser/sessionsManagementService.js';
import './media/forgeChats.css';

registerSingleton(IForgeChatService, ForgeChatService, InstantiationType.Delayed);
registerSingleton(ISessionsManagementService, SessionsManagementService, InstantiationType.Delayed);

const viewContainer = Registry.as<IViewContainersRegistry>(ViewExtensions.ViewContainersRegistry).registerViewContainer({
	id: FORGE_CHATS_VIEWLET_ID,
	title: nls.localize2('forgeChats', 'Chats'),
	ctorDescriptor: new SyncDescriptor(ForgeChatsViewPaneContainer),
	icon: forgeChatsViewIcon,
	order: 10,
	openCommandActionDescriptor: {
		id: FORGE_CHATS_VIEWLET_ID,
		mnemonicTitle: nls.localize({ key: 'miViewForgeChats', comment: ['&& denotes a mnemonic'] }, '&&Chats'),
		order: 10,
	},
}, ViewContainerLocation.Sidebar);

const chatsViewDescriptor: IViewDescriptor = {
	id: FORGE_CHATS_VIEW_ID,
	containerIcon: forgeChatsViewIcon,
	name: nls.localize2('forgeChats.view', 'Chats'),
	ctorDescriptor: new SyncDescriptor(ForgeChatsView),
	order: 1,
	canToggleVisibility: false,
	canMoveView: false,
};

Registry.as<IViewsRegistry>(ViewExtensions.ViewsRegistry).registerViews([chatsViewDescriptor], viewContainer);
