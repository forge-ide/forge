/*---------------------------------------------------------------------------------------------
 * Forge - ForgeChatService (browser)
 *--------------------------------------------------------------------------------------------*/

import { Emitter, Event } from '../../../../base/common/event.js';
import { Disposable } from '../../../../base/common/lifecycle.js';
import { URI } from '../../../../base/common/uri.js';
import { ILogService } from '../../../../platform/log/common/log.js';
import { IStorageService, StorageScope, StorageTarget } from '../../../../platform/storage/common/storage.js';
import { ForgeChatEntry, IForgeChatService } from '../common/forgeChatService.js';

const STORAGE_KEY = 'forge.chats.metadata';

export class ForgeChatService extends Disposable implements IForgeChatService {
	declare readonly _serviceBrand: undefined;

	private readonly _onDidChangeChats = this._register(new Emitter<void>());
	readonly onDidChangeChats: Event<void> = this._onDidChangeChats.event;

	private _chats: Map<string, ForgeChatEntry> = new Map();

	constructor(
		@IStorageService private readonly _storageService: IStorageService,
		@ILogService private readonly _logService: ILogService,
	) {
		super();
		this._loadFromStorage();
	}

	getChats(): ForgeChatEntry[] {
		return Array.from(this._chats.values())
			.sort((a, b) => b.lastActiveAt - a.lastActiveAt);
	}

	getChatsByProvider(providerName: string): ForgeChatEntry[] {
		return this.getChats().filter(c => c.providerName === providerName);
	}

	renameChat(resource: URI, name: string): void {
		const key = resource.toString();
		const existing = this._chats.get(key);
		if (!existing) { return; }
		this._chats.set(key, { ...existing, label: name });
		this._persist();
		this._onDidChangeChats.fire();
	}

	deleteChat(resource: URI): void {
		const key = resource.toString();
		if (!this._chats.has(key)) { return; }
		this._chats.delete(key);
		this._persist();
		this._onDidChangeChats.fire();
	}

	updateChatMetadata(resource: URI, patch: Partial<Omit<ForgeChatEntry, 'resource'>>): void {
		const key = resource.toString();
		const existing = this._chats.get(key);
		const updated: ForgeChatEntry = {
			resource,
			providerName: patch.providerName ?? existing?.providerName ?? 'unknown',
			label: patch.label ?? existing?.label ?? 'Untitled Chat',
			currentModel: patch.currentModel ?? existing?.currentModel ?? '',
			messageCount: patch.messageCount ?? existing?.messageCount ?? 0,
			lastActiveAt: patch.lastActiveAt ?? existing?.lastActiveAt ?? Date.now(),
			lastMessageSnippet: patch.lastMessageSnippet ?? existing?.lastMessageSnippet ?? '',
		};
		this._chats.set(key, updated);
		this._persist();
		this._onDidChangeChats.fire();
	}

	/**
	 * Called by the sessions contrib layer (which can import vs/sessions) when a session becomes active.
	 * ForgeChatService intentionally does not import vs/sessions to respect layering rules.
	 */
	notifySessionActivated(session: { resource: URI; providerType: string; label?: string }): void {
		const key = session.resource.toString();
		const existing = this._chats.get(key);
		this._chats.set(key, {
			resource: session.resource,
			providerName: session.providerType ?? existing?.providerName ?? 'unknown',
			label: existing?.label ?? session.label ?? 'Untitled Chat',
			currentModel: existing?.currentModel ?? '',
			messageCount: existing?.messageCount ?? 0,
			lastActiveAt: Date.now(),
			lastMessageSnippet: existing?.lastMessageSnippet ?? '',
		});
		this._persist();
		this._onDidChangeChats.fire();
	}

	openNewChat(): void {
		// Wired by the sessions contrib layer via ISessionsManagementService.openNewSessionView().
		// ForgeChatService does not import vs/sessions to respect workbench/services layer rules.
	}

	private _persist(): void {
		const serializable = Array.from(this._chats.values()).map(e => ({
			...e,
			resource: e.resource.toString(),
		}));
		try {
			this._storageService.store(STORAGE_KEY, JSON.stringify(serializable), StorageScope.WORKSPACE, StorageTarget.MACHINE);
		} catch (e) {
			this._logService.warn('ForgeChatService: failed to persist chat metadata', e);
		}
	}

	private _loadFromStorage(): void {
		const raw = this._storageService.get(STORAGE_KEY, StorageScope.WORKSPACE);
		if (!raw) { return; }
		try {
			const entries = JSON.parse(raw) as Array<Omit<ForgeChatEntry, 'resource'> & { resource: string }>;
			for (const entry of entries) {
				const resource = URI.parse(entry.resource);
				this._chats.set(resource.toString(), { ...entry, resource });
			}
		} catch (e) {
			this._logService.warn('ForgeChatService: failed to load chat metadata from storage', e);
		}
	}
}
