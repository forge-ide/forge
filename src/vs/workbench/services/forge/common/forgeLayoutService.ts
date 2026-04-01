/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { createDecorator } from '../../../../platform/instantiation/common/instantiation.js';
import { Event } from '../../../../base/common/event.js';

export type ForgeLayout = 'focus' | 'split' | 'quad' | 'code+ai';
export type PanePosition = 'tl' | 'tr' | 'bl' | 'br';

export interface ForgePaneState {
	readonly position: PanePosition;
	readonly providerId: string;
	readonly conversationId: string;
	readonly model?: string;
}

export interface ForgeLayoutState {
	readonly layout: ForgeLayout;
	readonly panes: ForgePaneState[];
}

export const IForgeLayoutService = createDecorator<IForgeLayoutService>('forgeLayoutService');

export interface IForgeLayoutService {
	readonly _serviceBrand: undefined;
	readonly onDidChangeLayout: Event<ForgeLayout>;
	readonly activeLayout: ForgeLayout;
	setLayout(layout: ForgeLayout): Promise<void>;
	openChatPane(position: PanePosition, providerId?: string): Promise<void>;
	getLayoutState(): ForgeLayoutState;
	saveLayout(): void;
	restoreLayout(): Promise<void>;
}
