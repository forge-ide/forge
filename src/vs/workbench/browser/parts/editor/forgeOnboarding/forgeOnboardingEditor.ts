/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { Dimension, IDomPosition } from '../../../../../base/browser/dom.js';
import { EditorPane } from '../editorPane.js';
import { IEditorGroup } from '../../../../services/editor/common/editorGroupsService.js';
import { ITelemetryService } from '../../../../../platform/telemetry/common/telemetry.js';
import { IThemeService } from '../../../../../platform/theme/common/themeService.js';
import { IStorageService } from '../../../../../platform/storage/common/storage.js';
import { IInstantiationService } from '../../../../../platform/instantiation/common/instantiation.js';
import { ForgeOnboardingView } from './forgeOnboardingView.js';

export class ForgeOnboardingEditor extends EditorPane {
	static readonly ID = 'workbench.editor.forgeOnboarding';

	private _view: ForgeOnboardingView | undefined;

	constructor(
		group: IEditorGroup,
		@ITelemetryService telemetryService: ITelemetryService,
		@IThemeService themeService: IThemeService,
		@IStorageService storageService: IStorageService,
		@IInstantiationService private readonly instantiationService: IInstantiationService,
	) {
		super(ForgeOnboardingEditor.ID, group, telemetryService, themeService, storageService);
	}

	protected createEditor(parent: HTMLElement): void {
		const container = document.createElement('div');
		container.style.width = '100%';
		container.style.height = '100%';
		parent.appendChild(container);

		const view = this.instantiationService.createInstance(ForgeOnboardingView, container);
		this._view = view;
		this._register(view);
	}

	override layout(dimension: Dimension, _position?: IDomPosition): void {
		this._view?.layout(dimension);
	}

	override dispose(): void {
		super.dispose();
	}
}
