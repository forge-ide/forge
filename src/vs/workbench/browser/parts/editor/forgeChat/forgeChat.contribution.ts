/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { localize, localize2 } from '../../../../../nls.js';
import { Registry } from '../../../../../platform/registry/common/platform.js';
import { EditorExtensions, IEditorFactoryRegistry, IEditorSerializer } from '../../../../common/editor.js';
import { EditorPaneDescriptor, IEditorPaneRegistry } from '../../../../browser/editor.js';
import { SyncDescriptor } from '../../../../../platform/instantiation/common/descriptors.js';
import { IInstantiationService, ServicesAccessor } from '../../../../../platform/instantiation/common/instantiation.js';
import { EditorInput } from '../../../../common/editor/editorInput.js';
import { IEditorService } from '../../../../services/editor/common/editorService.js';
import { generateUuid } from '../../../../../base/common/uuid.js';
import { Action2, registerAction2 } from '../../../../../platform/actions/common/actions.js';
import { IAIProviderService } from '../../../../../platform/ai/common/aiProviderService.js';
import { IForgeConfigService } from '../../../../services/forge/common/forgeConfigService.js';
import { ForgeChatEditorPane } from './forgeChatEditor.js';
import { ForgeChatInput } from './forgeChatInput.js';
import './forgeChatView.css';

// --- Editor Pane Registration ---

Registry.as<IEditorPaneRegistry>(EditorExtensions.EditorPane).registerEditorPane(
	EditorPaneDescriptor.create(
		ForgeChatEditorPane,
		ForgeChatEditorPane.ID,
		localize('forgeChat', "Forge Chat"),
	),
	[
		new SyncDescriptor(ForgeChatInput),
	],
);

// --- Editor Serializer ---

class ForgeChatInputSerializer implements IEditorSerializer {
	canSerialize(editor: EditorInput): boolean {
		return editor instanceof ForgeChatInput;
	}

	serialize(editor: EditorInput): string | undefined {
		if (!(editor instanceof ForgeChatInput)) {
			return undefined;
		}
		return JSON.stringify({
			providerName: editor.providerName,
			conversationId: editor.conversationId,
		});
	}

	deserialize(instantiationService: IInstantiationService, serializedEditor: string): EditorInput | undefined {
		try {
			const data = JSON.parse(serializedEditor) as { providerName: string; conversationId: string };
			return new ForgeChatInput(data.providerName, data.conversationId);
		} catch {
			return undefined;
		}
	}
}

Registry.as<IEditorFactoryRegistry>(EditorExtensions.EditorFactory)
	.registerEditorSerializer(ForgeChatInput.ID, ForgeChatInputSerializer);

// --- Commands ---

registerAction2(class extends Action2 {
	constructor() {
		super({
			id: 'forge.chat.new',
			title: localize2('forgeChat.new', "Forge: New Chat Pane"),
			f1: true,
		});
	}

	run(accessor: ServicesAccessor): void {
		const editorService = accessor.get(IEditorService);
		const aiProviderService = accessor.get(IAIProviderService);
		const forgeConfig = accessor.get(IForgeConfigService);
		const providerName = aiProviderService.getActiveProvider()?.name ?? forgeConfig.getConfig().provider;
		const input = new ForgeChatInput(providerName, generateUuid());
		editorService.openEditor(input, { pinned: true });
	}
});
