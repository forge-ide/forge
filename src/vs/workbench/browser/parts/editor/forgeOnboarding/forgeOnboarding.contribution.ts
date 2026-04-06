/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { localize, localize2 } from '../../../../../nls.js';
import { Disposable } from '../../../../../base/common/lifecycle.js';
import { Registry } from '../../../../../platform/registry/common/platform.js';
import { EditorExtensions, IEditorFactoryRegistry, IEditorSerializer } from '../../../../common/editor.js';
import { EditorPaneDescriptor, IEditorPaneRegistry } from '../../../../browser/editor.js';
import { SyncDescriptor } from '../../../../../platform/instantiation/common/descriptors.js';
import { IInstantiationService, ServicesAccessor } from '../../../../../platform/instantiation/common/instantiation.js';
import { EditorInput } from '../../../../common/editor/editorInput.js';
import { IEditorService } from '../../../../services/editor/common/editorService.js';
import { Action2, registerAction2 } from '../../../../../platform/actions/common/actions.js';
import { IWorkbenchContribution, WorkbenchPhase, registerWorkbenchContribution2 } from '../../../../common/contributions.js';
import { IForgeOnboardingService } from '../../../../services/forge/common/forgeOnboardingService.js';
import { ForgeOnboardingEditor } from './forgeOnboardingEditor.js';
import { ForgeOnboardingInput } from './forgeOnboardingInput.js';

// --- Editor Pane Registration ---

Registry.as<IEditorPaneRegistry>(EditorExtensions.EditorPane).registerEditorPane(
	EditorPaneDescriptor.create(
		ForgeOnboardingEditor,
		ForgeOnboardingEditor.ID,
		localize('forgeOnboarding', "Forge Onboarding"),
	),
	[
		new SyncDescriptor(ForgeOnboardingInput),
	],
);

// --- Editor Serializer ---

class ForgeOnboardingInputSerializer implements IEditorSerializer {
	canSerialize(_editor: EditorInput): boolean {
		return true;
	}

	serialize(_editor: EditorInput): string {
		return '{}';
	}

	deserialize(_instantiationService: IInstantiationService, _serializedEditor: string): EditorInput {
		return new ForgeOnboardingInput();
	}
}

Registry.as<IEditorFactoryRegistry>(EditorExtensions.EditorFactory)
	.registerEditorSerializer(ForgeOnboardingInput.ID, ForgeOnboardingInputSerializer);

// --- Commands ---

registerAction2(class extends Action2 {
	constructor() {
		super({
			id: 'forge.onboarding.open',
			title: localize2('forgeOnboarding.open', "Forge: Open Welcome"),
			f1: true,
		});
	}

	run(accessor: ServicesAccessor): void {
		const editorService = accessor.get(IEditorService);
		editorService.openEditor(new ForgeOnboardingInput(), { pinned: true });
	}
});

// --- First-launch trigger ---

class ForgeOnboardingBootstrap extends Disposable implements IWorkbenchContribution {
	static readonly ID = 'forge.onboarding.bootstrap';

	constructor(
		@IForgeOnboardingService onboardingService: IForgeOnboardingService,
		@IEditorService editorService: IEditorService,
	) {
		super();
		if (!onboardingService.onboardingComplete) {
			editorService.openEditor(new ForgeOnboardingInput(), { pinned: true });
		}
	}
}

registerWorkbenchContribution2(ForgeOnboardingBootstrap.ID, ForgeOnboardingBootstrap, WorkbenchPhase.AfterRestored);
