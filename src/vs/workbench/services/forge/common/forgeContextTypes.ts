/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { URI } from '../../../../base/common/uri.js';
import type { PanePosition } from './forgeLayoutService.js';

export type ForgeContextType = 'file' | 'selection' | 'gitDiff' | 'symbol' | 'paneHistory' | 'activeEditor';
export const ForgeContextType = {
	File: 'file' as const,
	Selection: 'selection' as const,
	GitDiff: 'gitDiff' as const,
	Symbol: 'symbol' as const,
	PaneHistory: 'paneHistory' as const,
	ActiveEditor: 'activeEditor' as const,
};

export interface ForgeContextItem {
	readonly type: ForgeContextType;
	readonly label: string;
	readonly detail?: string;
	readonly content: string;
	readonly tokenEstimate: number;
	readonly uri?: URI;
	readonly sourcePanePosition?: PanePosition;
}

export interface ForgeContextChip {
	readonly item: ForgeContextItem;
	readonly automatic: boolean;
}

export interface IForgeContextBudget {
	readonly maxTokens: number;
	readonly usedTokens: number;
	readonly items: ForgeContextChip[];
	readonly droppedCount: number;
}

export function formatContextItem(item: ForgeContextItem): string {
	switch (item.type) {
		case ForgeContextType.File:
			return `<file path="${item.label}">${item.content}</file>`;
		case ForgeContextType.Selection:
			return `<selection file="${item.label}" line="${item.detail ?? ''}">${item.content}</selection>`;
		case ForgeContextType.GitDiff:
			return `<diff>${item.content}</diff>`;
		case ForgeContextType.Symbol:
			return `<symbol name="${item.label}" file="${item.detail ?? ''}">${item.content}</symbol>`;
		case ForgeContextType.PaneHistory:
			return `<context pane="${item.label}">${item.content}</context>`;
		case ForgeContextType.ActiveEditor:
			return `<file path="${item.label}">${item.content}</file>`;
		default:
			return item.content;
	}
}
