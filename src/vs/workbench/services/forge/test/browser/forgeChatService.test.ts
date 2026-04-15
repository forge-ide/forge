import assert from 'assert';
import { Emitter } from '../../../../../base/common/event.js';
import { DisposableStore } from '../../../../../base/common/lifecycle.js';
import { URI } from '../../../../../base/common/uri.js';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import { ForgeChatService } from '../../browser/forgeChatService.js';
import { NullLogService } from '../../../../../platform/log/common/log.js';
import { IStorageService, StorageScope, StorageTarget } from '../../../../../platform/storage/common/storage.js';

function makeUri(id: string): URI {
	return URI.parse(`forge-session://${id}`);
}

function createMockStorageService(ds: DisposableStore): IStorageService {
	const store = new Map<string, string>();
	return {
		_serviceBrand: undefined,
		get: (key: string) => store.get(key),
		store: (key: string, value: string) => store.set(key, value),
		remove: (key: string) => store.delete(key),
		onDidChangeValue: ds.add(new Emitter<{ key: string; scope: StorageScope }>()).event,
		onDidFlush: ds.add(new Emitter<void>()).event,
		onWillSaveState: ds.add(new Emitter<void>()).event,
		flush: () => Promise.resolve(),
		keys: (_scope: StorageScope, _target: StorageTarget) => [],
		log: () => { },
		isNew: (_scope: StorageScope) => false,
		optimize: (_scope: StorageScope) => Promise.resolve(),
		switch: () => Promise.resolve(),
		hasScope: () => false,
		getBoolean: (_key: string, _scope: StorageScope, _fallback: boolean) => _fallback,
		getNumber: (_key: string, _scope: StorageScope, _fallback: number) => _fallback,
		getObject: (_key: string, _scope: StorageScope) => undefined,
	} as unknown as IStorageService;
}

suite('ForgeChatService', () => {
	let disposables: DisposableStore;

	setup(() => {
		disposables = new DisposableStore();
	});

	teardown(() => {
		disposables.dispose();
	});

	ensureNoDisposablesAreLeakedInTestSuite();

	function createService() {
		const storage = createMockStorageService(disposables);
		const svc = disposables.add(new ForgeChatService(
			storage,
			new NullLogService(),
		));
		return { svc, storage };
	}

	test('getChats returns empty list initially', () => {
		const { svc } = createService();
		assert.deepStrictEqual(svc.getChats(), []);
	});

	test('updateChatMetadata adds a new chat entry', () => {
		const { svc } = createService();
		const resource = makeUri('session-1');
		svc.updateChatMetadata(resource, {
			providerName: 'anthropic',
			label: 'My Chat',
			currentModel: 'claude-opus-4',
			messageCount: 3,
			lastActiveAt: 1000,
			lastMessageSnippet: 'Hello world',
		});
		const chats = svc.getChats();
		assert.strictEqual(chats.length, 1);
		assert.strictEqual(chats[0].label, 'My Chat');
		assert.strictEqual(chats[0].providerName, 'anthropic');
	});

	test('getChatsByProvider filters by provider', () => {
		const { svc } = createService();
		svc.updateChatMetadata(makeUri('s1'), { providerName: 'anthropic', label: 'A', currentModel: 'm', messageCount: 0, lastActiveAt: 1, lastMessageSnippet: '' });
		svc.updateChatMetadata(makeUri('s2'), { providerName: 'openai', label: 'B', currentModel: 'm', messageCount: 0, lastActiveAt: 2, lastMessageSnippet: '' });
		assert.strictEqual(svc.getChatsByProvider('anthropic').length, 1);
		assert.strictEqual(svc.getChatsByProvider('openai').length, 1);
		assert.strictEqual(svc.getChatsByProvider('vertex').length, 0);
	});

	test('renameChat updates label', () => {
		const { svc } = createService();
		const resource = makeUri('s1');
		svc.updateChatMetadata(resource, { providerName: 'anthropic', label: 'Old Name', currentModel: 'm', messageCount: 0, lastActiveAt: 1, lastMessageSnippet: '' });
		svc.renameChat(resource, 'New Name');
		assert.strictEqual(svc.getChats()[0].label, 'New Name');
	});

	test('deleteChat removes entry', () => {
		const { svc } = createService();
		const resource = makeUri('s1');
		svc.updateChatMetadata(resource, { providerName: 'anthropic', label: 'Chat', currentModel: 'm', messageCount: 0, lastActiveAt: 1, lastMessageSnippet: '' });
		assert.strictEqual(svc.getChats().length, 1);
		svc.deleteChat(resource);
		assert.strictEqual(svc.getChats().length, 0);
	});

	test('getChats sorted by lastActiveAt descending', () => {
		const { svc } = createService();
		svc.updateChatMetadata(makeUri('s1'), { providerName: 'anthropic', label: 'Old', currentModel: 'm', messageCount: 0, lastActiveAt: 1000, lastMessageSnippet: '' });
		svc.updateChatMetadata(makeUri('s2'), { providerName: 'anthropic', label: 'New', currentModel: 'm', messageCount: 0, lastActiveAt: 2000, lastMessageSnippet: '' });
		const chats = svc.getChats();
		assert.strictEqual(chats[0].label, 'New');
		assert.strictEqual(chats[1].label, 'Old');
	});

	test('onDidChangeChats fires after updateChatMetadata', () => {
		const { svc } = createService();
		let fired = 0;
		disposables.add(svc.onDidChangeChats(() => fired++));
		svc.updateChatMetadata(makeUri('s1'), { providerName: 'anthropic', label: 'A', currentModel: 'm', messageCount: 0, lastActiveAt: 1, lastMessageSnippet: '' });
		assert.strictEqual(fired, 1);
	});

	test('onDidChangeChats fires after deleteChat', () => {
		const { svc } = createService();
		const resource = makeUri('s1');
		svc.updateChatMetadata(resource, { providerName: 'anthropic', label: 'A', currentModel: 'm', messageCount: 0, lastActiveAt: 1, lastMessageSnippet: '' });
		let fired = 0;
		disposables.add(svc.onDidChangeChats(() => fired++));
		svc.deleteChat(resource);
		assert.strictEqual(fired, 1);
	});
});
