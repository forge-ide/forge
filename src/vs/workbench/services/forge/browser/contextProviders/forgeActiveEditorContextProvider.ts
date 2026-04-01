/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { Disposable } from '../../../../../base/common/lifecycle.js';
import { Emitter } from '../../../../../base/common/event.js';
import { IEditorService } from '../../../../services/editor/common/editorService.js';
import { IConfigurationService } from '../../../../../platform/configuration/common/configuration.js';
import { IFileService } from '../../../../../platform/files/common/files.js';
import { ILogService } from '../../../../../platform/log/common/log.js';
import { IForgeLayoutService, ForgeLayout, PanePosition } from '../../common/forgeLayoutService.js';
import { IForgeContextService } from '../../common/forgeContextService.js';
import { ForgeContextItem, ForgeContextType } from '../../common/forgeContextTypes.js';
import { ForgeChatInput } from '../../../../browser/parts/editor/forgeChat/forgeChatInput.js';
import { IWorkbenchContribution, registerWorkbenchContribution2, WorkbenchPhase } from '../../../../common/contributions.js';

export class ForgeActiveEditorContextProvider extends Disposable implements IWorkbenchContribution {

	static readonly ID = 'workbench.contrib.forgeActiveEditorContext';

	private readonly _onDidChangeActiveContext = this._register(new Emitter<ForgeContextItem | undefined>());
	readonly onDidChangeActiveContext = this._onDidChangeActiveContext.event;

	private currentItem: ForgeContextItem | undefined;
	private lastPanePosition: PanePosition | undefined;

	constructor(
		@IEditorService private readonly editorService: IEditorService,
		@IForgeLayoutService private readonly layoutService: IForgeLayoutService,
		@IForgeContextService private readonly contextService: IForgeContextService,
		@IFileService private readonly fileService: IFileService,
		@IConfigurationService private readonly configService: IConfigurationService,
		@ILogService private readonly logService: ILogService,
	) {
		super();

		this._register(this.editorService.onDidActiveEditorChange(() => {
			if (this.configService.getValue<boolean>('forge.autoAttachActiveEditor')) {
				this.updateActiveEditorContext();
			}
		}));

		this._register(this.configService.onDidChangeConfiguration(e => {
			if (e.affectsConfiguration('forge.autoAttachActiveEditor')) {
				if (!this.configService.getValue<boolean>('forge.autoAttachActiveEditor')) {
					this.clearAutomaticContext();
				}
			}
		}));
	}

	private async updateActiveEditorContext(): Promise<void> {
		const activeEditor = this.editorService.activeEditor;

		// Don't attach chat panes as context
		if (activeEditor instanceof ForgeChatInput) {
			return;
		}

		const resource = activeEditor?.resource;
		if (!resource) {
			this.clearAutomaticContext();
			return;
		}

		try {
			const fileContent = await this.fileService.readFile(resource);
			const text = fileContent.value.toString();
			const fileName = resource.path.split('/').pop() ?? resource.path;

			const item: ForgeContextItem = {
				type: ForgeContextType.ActiveEditor,
				label: fileName,
				detail: resource.path,
				content: text,
				tokenEstimate: Math.ceil(text.length / 4),
				uri: resource,
			};

			const panePosition = this.findNearestAIPane();
			if (!panePosition) {
				return;
			}

			// Remove the previously attached automatic context
			if (this.currentItem && this.lastPanePosition) {
				this.contextService.removeContextChip(this.lastPanePosition, this.currentItem);
			}

			this.contextService.addContextChip(panePosition, item, true);
			this.currentItem = item;
			this.lastPanePosition = panePosition;
			this._onDidChangeActiveContext.fire(item);

			this.logService.debug(`[ForgeActiveEditorContext] Attached active editor context: ${fileName}`);
		} catch (error) {
			this.logService.warn('[ForgeActiveEditorContext] Failed to read active editor file', error);
		}
	}

	private clearAutomaticContext(): void {
		if (this.currentItem && this.lastPanePosition) {
			this.contextService.removeContextChip(this.lastPanePosition, this.currentItem);
		}
		this.currentItem = undefined;
		this.lastPanePosition = undefined;
		this._onDidChangeActiveContext.fire(undefined);
	}

	private findNearestAIPane(): PanePosition | undefined {
		const layout: ForgeLayout = this.layoutService.activeLayout;

		switch (layout) {
			case 'code+ai':
				// Right pane is always AI in code+ai layout
				return 'tr';
			case 'quad': {
				// In quad layout, determine based on active editor position
				const state = this.layoutService.getLayoutState();
				const panes = state.panes;
				// Right column panes are typically AI; prefer top-right
				if (panes.some(p => p.position === 'tr')) {
					return 'tr';
				}
				if (panes.some(p => p.position === 'br')) {
					return 'br';
				}
				return undefined;
			}
			case 'focus':
				// Single pane — can't auto-attach to self
				return undefined;
			case 'split':
				// Ambiguous which pane is AI
				return undefined;
			default:
				return undefined;
		}
	}
}

registerWorkbenchContribution2(
	ForgeActiveEditorContextProvider.ID,
	ForgeActiveEditorContextProvider,
	WorkbenchPhase.AfterRestored,
);
