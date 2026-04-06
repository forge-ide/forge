/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { EditorInput } from '../../../../common/editor/editorInput.js';
import { EditorInputCapabilities, IUntypedEditorInput } from '../../../../common/editor.js';
import { URI } from '../../../../../base/common/uri.js';

export class ForgeOnboardingInput extends EditorInput {
	static readonly ID = 'workbench.input.forgeOnboarding';
	static readonly RESOURCE = URI.parse('forge://onboarding/welcome');

	override get typeId(): string { return ForgeOnboardingInput.ID; }
	override get editorId(): string | undefined { return ForgeOnboardingInput.ID; }
	override get resource(): URI { return ForgeOnboardingInput.RESOURCE; }
	override get capabilities(): EditorInputCapabilities { return EditorInputCapabilities.Readonly; }
	override getName(): string { return 'Welcome to Forge'; }

	override matches(other: EditorInput | IUntypedEditorInput): boolean {
		return other instanceof ForgeOnboardingInput;
	}
}
