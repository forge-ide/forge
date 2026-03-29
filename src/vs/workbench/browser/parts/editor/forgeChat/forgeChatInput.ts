/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { EditorInput } from '../../../../common/editor/editorInput.js';
import { EditorInputCapabilities, IUntypedEditorInput } from '../../../../common/editor.js';
import { URI } from '../../../../../base/common/uri.js';

const FORGE_CHAT_SCHEME = 'forge-chat';

export class ForgeChatInput extends EditorInput {
	static readonly ID = 'forge.chatInput';

	private readonly _uri: URI;

	constructor(
		readonly providerName: string,
		readonly conversationId: string,
	) {
		super();
		this._uri = URI.from({ scheme: FORGE_CHAT_SCHEME, path: `/${conversationId}` });
	}

	override get typeId(): string { return ForgeChatInput.ID; }
	override get editorId(): string | undefined { return ForgeChatInput.ID; }
	override get resource(): URI { return this._uri; }
	override get capabilities(): EditorInputCapabilities {
		return EditorInputCapabilities.Readonly | EditorInputCapabilities.Singleton;
	}

	override getName(): string {
		return `Chat — ${this.providerName}`;
	}

	override matches(otherInput: EditorInput | IUntypedEditorInput): boolean {
		if (otherInput instanceof ForgeChatInput) {
			return otherInput.conversationId === this.conversationId;
		}
		return super.matches(otherInput);
	}
}
