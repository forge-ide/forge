/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { createDecorator } from '../../../../platform/instantiation/common/instantiation.js';
import { Event } from '../../../../base/common/event.js';
import { CancellationToken } from '../../../../base/common/cancellation.js';
import type { PanePosition } from './forgeLayoutService.js';
import type { ForgeContextChip, ForgeContextItem, IForgeContextBudget } from './forgeContextTypes.js';

export const IForgeContextService = createDecorator<IForgeContextService>('forgeContextService');

export interface IForgeContextService {
	readonly _serviceBrand: undefined;
	readonly onDidChangeContext: Event<PanePosition | undefined>;
	getContextChips(panePosition: PanePosition | undefined): ForgeContextChip[];
	addContextChip(panePosition: PanePosition | undefined, item: ForgeContextItem, automatic?: boolean): void;
	removeContextChip(panePosition: PanePosition | undefined, item: ForgeContextItem): void;
	clearContext(panePosition: PanePosition | undefined): void;
	resolveContextPrompt(panePosition: PanePosition | undefined, maxTokens: number, token?: CancellationToken): Promise<IForgeContextBudget>;
	showContextPicker(panePosition: PanePosition | undefined): Promise<ForgeContextItem[]>;
}
