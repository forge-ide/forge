/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { Dimension, IDomPosition } from '../../../../../base/browser/dom.js';
import { CancellationToken } from '../../../../../base/common/cancellation.js';
import { DisposableStore } from '../../../../../base/common/lifecycle.js';
import { IEditorOpenContext } from '../../../../common/editor.js';
import { EditorPane } from '../editorPane.js';
import { IEditorGroup } from '../../../../services/editor/common/editorGroupsService.js';
import { ITelemetryService } from '../../../../../platform/telemetry/common/telemetry.js';
import { IThemeService } from '../../../../../platform/theme/common/themeService.js';
import { IStorageService } from '../../../../../platform/storage/common/storage.js';
import { IEditorOptions } from '../../../../../platform/editor/common/editor.js';
import { IInstantiationService } from '../../../../../platform/instantiation/common/instantiation.js';
import { IForgeConfigService } from '../../../../services/forge/common/forgeConfigService.js';
import { ForgeChatInput } from './forgeChatInput.js';
import { ForgeChatView } from './forgeChatView.js';

export class ForgeChatEditorPane extends EditorPane {
	static readonly ID = 'forge.chatEditorPane';

	private container: HTMLElement | undefined;
	private chatView: ForgeChatView | undefined;
	private readonly _inputListeners = this._register(new DisposableStore());

	constructor(
		group: IEditorGroup,
		@ITelemetryService telemetryService: ITelemetryService,
		@IThemeService themeService: IThemeService,
		@IStorageService storageService: IStorageService,
		@IInstantiationService private readonly instantiationService: IInstantiationService,
		@IForgeConfigService private readonly forgeConfigService: IForgeConfigService,
	) {
		super(ForgeChatEditorPane.ID, group, telemetryService, themeService, storageService);
	}

	protected createEditor(parent: HTMLElement): void {
		this.container = document.createElement('div');
		this.container.style.width = '100%';
		this.container.style.height = '100%';
		parent.appendChild(this.container);

		this.chatView = this._register(this.instantiationService.createInstance(ForgeChatView, this.container));
	}

	override async setInput(
		input: ForgeChatInput,
		options: IEditorOptions | undefined,
		context: IEditorOpenContext,
		token: CancellationToken,
	): Promise<void> {
		await super.setInput(input, options, context, token);
		this.chatView?.setConversation(input.conversationId, input.providerName);

		// Keep tab title in sync when forge.json config loads or changes
		this._inputListeners.clear();
		this._inputListeners.add(this.forgeConfigService.onDidChange(config => {
			input.setProviderName(config.provider);
		}));
	}

	override layout(dimension: Dimension, _position?: IDomPosition): void {
		if (this.container) {
			this.container.style.width = `${dimension.width}px`;
			this.container.style.height = `${dimension.height}px`;
		}
	}
}
