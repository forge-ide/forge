/*---------------------------------------------------------------------------------------------
 * Forge - MCP Servers activity bar tab
 *--------------------------------------------------------------------------------------------*/

import * as nls from '../../../../nls.js';
import { Registry } from '../../../../platform/registry/common/platform.js';
import { SyncDescriptor } from '../../../../platform/instantiation/common/descriptors.js';
import { registerSingleton, InstantiationType } from '../../../../platform/instantiation/common/extensions.js';
import { Extensions as ViewExtensions, IViewContainersRegistry, IViewDescriptor, IViewsRegistry, ViewContainerLocation } from '../../../common/views.js';
import { ForgeMcpViewPaneContainer, forgeMcpViewIcon, FORGE_MCP_VIEWLET_ID } from './forgeMcpViewlet.js';
import { ForgeMcpView, FORGE_MCP_VIEW_ID } from './forgeMcpView.js';
import { IForgeMcpBridgeHost } from '../../../services/forge/browser/forgeMcpService.js';
import { ForgeMcpBridgeHost } from '../../forgeAI/browser/forgeMcpBridgeHost.js';
import './media/forgeMcp.css';

// Bridge between VS Code's IMcpService (contrib/mcp) and ForgeMcpService (services/forge).
registerSingleton(IForgeMcpBridgeHost, ForgeMcpBridgeHost, InstantiationType.Delayed);

const viewContainer = Registry.as<IViewContainersRegistry>(ViewExtensions.ViewContainersRegistry).registerViewContainer({
	id: FORGE_MCP_VIEWLET_ID,
	title: nls.localize2('forgeMcp', 'MCP Servers'),
	ctorDescriptor: new SyncDescriptor(ForgeMcpViewPaneContainer),
	icon: forgeMcpViewIcon,
	order: 12,
	openCommandActionDescriptor: {
		id: FORGE_MCP_VIEWLET_ID,
		mnemonicTitle: nls.localize({ key: 'miViewForgeMcp', comment: ['&& denotes a mnemonic'] }, '&&MCP Servers'),
		order: 12,
	},
}, ViewContainerLocation.Sidebar);

const mcpViewDescriptor: IViewDescriptor = {
	id: FORGE_MCP_VIEW_ID,
	containerIcon: forgeMcpViewIcon,
	name: nls.localize2('forgeMcp.view', 'MCP Servers'),
	ctorDescriptor: new SyncDescriptor(ForgeMcpView),
	order: 1,
	canToggleVisibility: false,
	canMoveView: false,
};

Registry.as<IViewsRegistry>(ViewExtensions.ViewsRegistry).registerViews([mcpViewDescriptor], viewContainer);
