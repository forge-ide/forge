/*---------------------------------------------------------------------------------------------
 * Forge - Skills activity bar tab
 *--------------------------------------------------------------------------------------------*/

import * as nls from '../../../../nls.js';
import { Registry } from '../../../../platform/registry/common/platform.js';
import { SyncDescriptor } from '../../../../platform/instantiation/common/descriptors.js';
import { Extensions as ViewExtensions, IViewContainersRegistry, IViewDescriptor, IViewsRegistry, ViewContainerLocation } from '../../../common/views.js';
import { ForgeSkillsViewPaneContainer, forgeSkillsViewIcon, FORGE_SKILLS_VIEWLET_ID } from './forgeSkillsViewlet.js';
import { ForgeSkillsView, FORGE_SKILLS_VIEW_ID } from './forgeSkillsView.js';
import './media/forgeSkills.css';

const viewContainer = Registry.as<IViewContainersRegistry>(ViewExtensions.ViewContainersRegistry).registerViewContainer({
	id: FORGE_SKILLS_VIEWLET_ID,
	title: nls.localize2('forgeSkills', 'Skills'),
	ctorDescriptor: new SyncDescriptor(ForgeSkillsViewPaneContainer),
	icon: forgeSkillsViewIcon,
	order: 13,
	openCommandActionDescriptor: {
		id: FORGE_SKILLS_VIEWLET_ID,
		mnemonicTitle: nls.localize({ key: 'miViewForgeSkills', comment: ['&& denotes a mnemonic'] }, '&&Skills'),
		order: 13,
	},
}, ViewContainerLocation.Sidebar);

const skillsViewDescriptor: IViewDescriptor = {
	id: FORGE_SKILLS_VIEW_ID,
	containerIcon: forgeSkillsViewIcon,
	name: nls.localize2('forgeSkills.view', 'Skills'),
	ctorDescriptor: new SyncDescriptor(ForgeSkillsView),
	order: 1,
	canToggleVisibility: false,
	canMoveView: false,
};

Registry.as<IViewsRegistry>(ViewExtensions.ViewsRegistry).registerViews([skillsViewDescriptor], viewContainer);
