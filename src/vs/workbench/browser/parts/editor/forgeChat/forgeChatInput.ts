/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { EditorInput } from '../../../../common/editor/editorInput.js';
import { EditorInputCapabilities, IUntypedEditorInput } from '../../../../common/editor.js';
import { URI } from '../../../../../base/common/uri.js';
import { PanePosition } from '../../../../services/forge/common/forgeLayoutService.js';

const FORGE_CHAT_SCHEME = 'forge-chat';

const PANE_POSITION_LABELS: Record<PanePosition, string> = {
	'tl': 'Top Left',
	'tr': 'Top Right',
	'bl': 'Bottom Left',
	'br': 'Bottom Right',
};

export class ForgeChatInput extends EditorInput {
	static readonly ID = 'forge.chatInput';

	private readonly _uri: URI;

	private _providerName: string;
	private _panePosition: PanePosition | undefined;
	private _model: string | undefined;

	hasHistory: boolean = false;

	get providerName(): string { return this._providerName; }

	setProviderName(name: string): void {
		if (name !== this._providerName) {
			this._providerName = name;
			this._onDidChangeLabel.fire();
		}
	}

	get panePosition(): PanePosition | undefined { return this._panePosition; }

	set panePosition(value: PanePosition | undefined) {
		if (value !== this._panePosition) {
			this._panePosition = value;
			this._onDidChangeLabel.fire();
		}
	}

	get model(): string | undefined { return this._model; }

	setModel(value: string): void {
		if (value !== this._model) {
			this._model = value;
			this._onDidChangeLabel.fire();
		}
	}

	constructor(
		providerName: string,
		readonly conversationId: string,
	) {
		super();
		this._providerName = providerName;
		this._uri = URI.from({ scheme: FORGE_CHAT_SCHEME, path: `/${conversationId}` });
	}

	override get typeId(): string { return ForgeChatInput.ID; }
	override get editorId(): string | undefined { return ForgeChatInput.ID; }
	override get resource(): URI { return this._uri; }
	override get capabilities(): EditorInputCapabilities {
		return EditorInputCapabilities.Readonly;
	}

	override getName(): string {
		const pos = this._panePosition ? ` (${PANE_POSITION_LABELS[this._panePosition]})` : '';
		return `Chat — ${this.providerName}${pos}`;
	}

	override matches(otherInput: EditorInput | IUntypedEditorInput): boolean {
		if (otherInput instanceof ForgeChatInput) {
			return otherInput.conversationId === this.conversationId;
		}
		return super.matches(otherInput);
	}
}
