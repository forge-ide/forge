/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import type { PanePosition, ForgeLayout, ForgePaneState } from './forgeLayoutService.js';

export interface SerializedConversation {
	readonly panePosition: PanePosition;
	readonly providerId: string;
	readonly conversationId: string;
	readonly messages: ReadonlyArray<{ readonly role: 'user' | 'assistant' | 'system'; readonly content: string }>;
}

export interface SerializedEditor {
	readonly uri: string;
}

export interface ForgeWorkspaceConfig {
	readonly id: string;
	readonly name: string;
	readonly createdAt: number;
	readonly layout: ForgeLayout;
	readonly panes: ForgePaneState[];
	readonly conversations: SerializedConversation[];
	readonly openEditors: SerializedEditor[];
}
