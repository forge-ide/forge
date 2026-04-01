/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { Disposable } from '../../../../../base/common/lifecycle.js';

import { ILogService } from '../../../../../platform/log/common/log.js';
import { IEditorGroupsService, GroupsOrder } from '../../../../services/editor/common/editorGroupsService.js';
import { IForgeLayoutService, PanePosition } from '../../common/forgeLayoutService.js';
import { ForgeContextItem, ForgeContextType } from '../../common/forgeContextTypes.js';
import { ForgeChatInput } from '../../../../browser/parts/editor/forgeChat/forgeChatInput.js';

const PANE_POSITION_LABELS: Record<PanePosition, string> = {
	'tl': 'TL',
	'tr': 'TR',
	'bl': 'BL',
	'br': 'BR',
};

export class ForgePaneHistoryContextProvider extends Disposable {

	constructor(
		@IForgeLayoutService private readonly layoutService: IForgeLayoutService,
		@IEditorGroupsService private readonly editorGroupsService: IEditorGroupsService,
		@ILogService private readonly logService: ILogService,
	) {
		super();
	}

	/**
	 * Returns context items representing available pane histories, excluding the
	 * given pane position (the requesting pane should not reference itself).
	 */
	getAvailablePaneHistories(excludePosition: PanePosition | undefined): ForgeContextItem[] {
		const state = this.layoutService.getLayoutState();
		const items: ForgeContextItem[] = [];

		for (const pane of state.panes) {
			if (pane.position === excludePosition) {
				continue;
			}

			// Verify the pane actually has a chat editor open
			if (!this.hasChatEditor(pane.position)) {
				continue;
			}

			const posLabel = PANE_POSITION_LABELS[pane.position];
			items.push({
				type: ForgeContextType.PaneHistory,
				label: `Pane ${posLabel} \u2014 ${pane.providerId}`,
				detail: `Conversation history from ${posLabel} pane`,
				content: '', // Resolved lazily during prompt building
				tokenEstimate: 0,
				sourcePanePosition: pane.position,
			});
		}

		return items;
	}

	/**
	 * Resolves a pane's conversation history into a context item.
	 * Content is left empty here; the ForgeChatView will fill it in
	 * during prompt building when it can access the target pane's messages.
	 */
	resolvePaneHistory(position: PanePosition): ForgeContextItem {
		const state = this.layoutService.getLayoutState();
		const paneState = state.panes.find(p => p.position === position);

		if (!paneState) {
			this.logService.debug(`[ForgePaneHistoryContextProvider] No pane state for position '${position}'`);
			return {
				type: ForgeContextType.PaneHistory,
				label: `Pane ${PANE_POSITION_LABELS[position]} \u2014 unavailable`,
				content: 'Pane no longer available',
				tokenEstimate: 0,
				sourcePanePosition: position,
			};
		}

		const posLabel = PANE_POSITION_LABELS[paneState.position];
		return {
			type: ForgeContextType.PaneHistory,
			label: `Pane ${posLabel} \u2014 ${paneState.providerId}`,
			detail: `Conversation history from ${posLabel} pane`,
			content: '', // Resolved by ForgeChatView during prompt building
			tokenEstimate: 0,
			sourcePanePosition: paneState.position,
		};
	}

	/**
	 * Checks whether a given pane position has a ForgeChatInput as its active editor.
	 */
	private hasChatEditor(position: PanePosition): boolean {
		const state = this.layoutService.getLayoutState();
		const paneState = state.panes.find(p => p.position === position);
		if (!paneState) {
			return false;
		}

		// Walk the editor groups to find the group for this position
		const groups = this.editorGroupsService.getGroups(GroupsOrder.GRID_APPEARANCE);
		const paneIndex = this.positionToIndex(position);
		if (paneIndex >= groups.length) {
			return false;
		}

		const group = groups[paneIndex];
		const activeEditor = group.activeEditor;
		return activeEditor instanceof ForgeChatInput;
	}

	private positionToIndex(position: PanePosition): number {
		switch (position) {
			case 'tl': return 0;
			case 'tr': return 1;
			case 'bl': return 2;
			case 'br': return 3;
		}
	}
}
