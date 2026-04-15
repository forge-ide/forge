/*---------------------------------------------------------------------------------------------
 * Forge - IForgeChatService
 *--------------------------------------------------------------------------------------------*/

import { Event } from '../../../../base/common/event.js';
import { URI } from '../../../../base/common/uri.js';
import { createDecorator } from '../../../../platform/instantiation/common/instantiation.js';

export const IForgeChatService = createDecorator<IForgeChatService>('forgeChatService');

/**
 * Metadata for a single chat session tracked by the panel.
 * Fields beyond `resource` and `providerName` are maintained by IForgeChatService
 * via IStorageService, since ISessionsManagementService does not expose them.
 */
export interface ForgeChatEntry {
	readonly resource: URI;
	readonly providerName: string;
	readonly label: string;            // User-visible name, editable via renameChat()
	readonly currentModel: string;     // Last active model in the session
	readonly messageCount: number;
	readonly lastActiveAt: number;     // Unix ms, for sorting
	readonly lastMessageSnippet: string;
}

export interface IForgeChatService {
	readonly _serviceBrand: undefined;

	/** Fires when the known chat list changes (added, renamed, deleted, metadata updated). */
	readonly onDidChangeChats: Event<void>;

	/** All known chats, sorted by lastActiveAt descending. */
	getChats(): ForgeChatEntry[];

	/** Chats for a specific provider, sorted by lastActiveAt descending. */
	getChatsByProvider(providerName: string): ForgeChatEntry[];

	/** Rename a chat. No-op if the resource is unknown. */
	renameChat(resource: URI, name: string): void;

	/** Soft-delete a chat. Removes it from getChats() results. */
	deleteChat(resource: URI): void;

	/**
	 * Update stored metadata for a session (model, message count, snippet, lastActiveAt).
	 * Called internally when the active session changes; exposed for testing.
	 */
	updateChatMetadata(resource: URI, patch: Partial<Omit<ForgeChatEntry, 'resource'>>): void;

	/** Open the new session view. Provider/model pre-selection is handled by the session UI. */
	openNewChat(): void;

	/**
	 * Notify the service that a session became active. Called by the sessions contrib layer
	 * (which can import vs/sessions) when `ISessionsManagementService.activeSession` changes.
	 * ForgeChatService does not import vs/sessions directly to respect layering rules.
	 */
	notifySessionActivated(session: { resource: URI; providerType: string; label?: string }): void;
}
