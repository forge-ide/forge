/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { KeyCode, KeyMod } from '../../../../../base/common/keyCodes.js';
import { localize, localize2 } from '../../../../../nls.js';
import { Registry } from '../../../../../platform/registry/common/platform.js';
import { Extensions as ConfigurationExtensions, IConfigurationRegistry } from '../../../../../platform/configuration/common/configurationRegistry.js';
import { EditorExtensions, IEditorFactoryRegistry, IEditorSerializer } from '../../../../common/editor.js';
import { EditorPaneDescriptor, IEditorPaneRegistry } from '../../../../browser/editor.js';
import { SyncDescriptor } from '../../../../../platform/instantiation/common/descriptors.js';
import { IInstantiationService, ServicesAccessor } from '../../../../../platform/instantiation/common/instantiation.js';
import { EditorInput } from '../../../../common/editor/editorInput.js';
import { IEditorService } from '../../../../services/editor/common/editorService.js';
import { generateUuid } from '../../../../../base/common/uuid.js';
import { Action2, registerAction2 } from '../../../../../platform/actions/common/actions.js';
import { KeybindingWeight } from '../../../../../platform/keybinding/common/keybindingsRegistry.js';
import { IAIProviderService } from '../../../../../platform/ai/common/aiProviderService.js';
import { IForgeConfigService } from '../../../../services/forge/common/forgeConfigService.js';
import { IForgeLayoutService, PanePosition } from '../../../../services/forge/common/forgeLayoutService.js';
import { ForgeChatEditorPane } from './forgeChatEditor.js';
import { ForgeChatInput } from './forgeChatInput.js';
import './forgeChatView.css';

// --- Context Providers ---
import '../../../../services/forge/browser/contextProviders/forgeActiveEditorContextProvider.js';

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

interface ForgeChatSerializedData {
	providerName: string;
	conversationId: string;
	panePosition?: string;
	model?: string;
}

class ForgeChatInputSerializer implements IEditorSerializer {
	canSerialize(editor: EditorInput): boolean {
		return editor instanceof ForgeChatInput;
	}

	serialize(editor: EditorInput): string | undefined {
		if (!(editor instanceof ForgeChatInput)) {
			return undefined;
		}
		const data: ForgeChatSerializedData = {
			providerName: editor.providerName,
			conversationId: editor.conversationId,
			panePosition: editor.panePosition,
			model: editor.model,
		};
		return JSON.stringify(data);
	}

	deserialize(instantiationService: IInstantiationService, serializedEditor: string): EditorInput | undefined {
		try {
			const data = JSON.parse(serializedEditor) as ForgeChatSerializedData;
			const input = new ForgeChatInput(data.providerName, data.conversationId);
			if (data.panePosition) {
				input.panePosition = data.panePosition as PanePosition;
			}
			if (data.model) {
				input.setModel(data.model);
			}
			return input;
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
		const providerName = aiProviderService.getDefaultProviderName() ?? forgeConfig.getConfig().defaultProvider;
		const input = new ForgeChatInput(providerName, generateUuid());
		editorService.openEditor(input, { pinned: true });
	}
});

// --- Layout Commands ---

registerAction2(class extends Action2 {
	constructor() {
		super({
			id: 'forge.layout.focus',
			title: localize2('forgeLayout.focus', "Forge: Focus Mode"),
			f1: true,
			keybinding: {
				primary: KeyMod.CtrlCmd | KeyMod.Shift | KeyCode.Digit1,
				weight: KeybindingWeight.WorkbenchContrib,
			},
		});
	}

	async run(accessor: ServicesAccessor): Promise<void> {
		await accessor.get(IForgeLayoutService).setLayout('focus');
	}
});

registerAction2(class extends Action2 {
	constructor() {
		super({
			id: 'forge.layout.split',
			title: localize2('forgeLayout.split', "Forge: Split Canvas"),
			f1: true,
			keybinding: {
				primary: KeyMod.CtrlCmd | KeyMod.Shift | KeyCode.Digit2,
				weight: KeybindingWeight.WorkbenchContrib,
			},
		});
	}

	async run(accessor: ServicesAccessor): Promise<void> {
		await accessor.get(IForgeLayoutService).setLayout('split');
	}
});

registerAction2(class extends Action2 {
	constructor() {
		super({
			id: 'forge.layout.codeai',
			title: localize2('forgeLayout.codeai', "Forge: Code + AI"),
			f1: true,
			keybinding: {
				primary: KeyMod.CtrlCmd | KeyMod.Shift | KeyCode.Digit3,
				weight: KeybindingWeight.WorkbenchContrib,
			},
		});
	}

	async run(accessor: ServicesAccessor): Promise<void> {
		await accessor.get(IForgeLayoutService).setLayout('code+ai');
	}
});

registerAction2(class extends Action2 {
	constructor() {
		super({
			id: 'forge.layout.quad',
			title: localize2('forgeLayout.quad', "Forge: Quad Canvas"),
			f1: true,
			keybinding: {
				primary: KeyMod.CtrlCmd | KeyMod.Shift | KeyCode.Digit4,
				weight: KeybindingWeight.WorkbenchContrib,
			},
		});
	}

	async run(accessor: ServicesAccessor): Promise<void> {
		await accessor.get(IForgeLayoutService).setLayout('quad');
	}
});

// --- Configuration ---

Registry.as<IConfigurationRegistry>(ConfigurationExtensions.Configuration).registerConfiguration({
	id: 'forge',
	title: localize('forgeConfigurationTitle', "Forge"),
	type: 'object',
	properties: {
		'forge.autoAttachActiveEditor': {
			type: 'boolean',
			default: false,
			description: localize('forge.autoAttachActiveEditor', "Automatically attach the active code editor as context to the nearest AI chat pane."),
		},
	},
});
