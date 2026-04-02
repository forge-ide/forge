/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import assert from 'assert';
import { Emitter, Event } from '../../../../../base/common/event.js';
import { VSBuffer } from '../../../../../base/common/buffer.js';
import { DisposableStore } from '../../../../../base/common/lifecycle.js';
import { Schemas } from '../../../../../base/common/network.js';
import { URI } from '../../../../../base/common/uri.js';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import { ConfigurationTarget, type IConfigurationChangeEvent } from '../../../../../platform/configuration/common/configuration.js';
import { FileService } from '../../../../../platform/files/common/fileService.js';
import { InMemoryFileSystemProvider } from '../../../../../platform/files/common/inMemoryFilesystemProvider.js';
import { NullLogService } from '../../../../../platform/log/common/log.js';
import type { IFileService } from '../../../../../platform/files/common/files.js';
import type { IEditorService } from '../../../../services/editor/common/editorService.js';
import { EditorInput } from '../../../../common/editor/editorInput.js';
import { ForgeChatInput } from '../../../../browser/parts/editor/forgeChat/forgeChatInput.js';
import { ForgeActiveEditorContextProvider } from '../../browser/contextProviders/forgeActiveEditorContextProvider.js';
import { ForgeContextType, type ForgeContextItem } from '../../common/forgeContextTypes.js';
import type { IForgeContextService } from '../../common/forgeContextService.js';
import type { IForgeLayoutService, ForgeLayout, ForgeLayoutState, PanePosition } from '../../common/forgeLayoutService.js';

// ---------------------------------------------------------------------------
// Mock helpers
// ---------------------------------------------------------------------------

/**
 * Minimal EditorInput subclass with a controllable resource URI.
 */
class MockEditorInput extends EditorInput {
	static readonly ID = 'mock.editorInput';

	constructor(private readonly _resource: URI | undefined) {
		super();
	}

	override get typeId(): string { return MockEditorInput.ID; }
	override get resource(): URI | undefined { return this._resource; }
	override getName(): string { return this._resource?.path.split('/').pop() ?? 'untitled'; }
}

function makeConfigChangeEvent(key: string): IConfigurationChangeEvent {
	return {
		source: ConfigurationTarget.USER,
		affectedKeys: new Set([key]),
		change: { keys: [key], overrides: [] },
		affectsConfiguration(configuration: string): boolean {
			return key === configuration || key.startsWith(configuration + '.');
		},
	};
}

interface SpyContextService extends IForgeContextService {
	readonly addContextChipCalls: Array<{ position: PanePosition | undefined; item: ForgeContextItem; automatic: boolean | undefined }>;
	readonly removeContextChipCalls: Array<{ position: PanePosition | undefined; item: ForgeContextItem }>;
	readonly clearContextCalls: Array<{ position: PanePosition | undefined }>;
}

function makeSpyContextService(): SpyContextService {
	const addContextChipCalls: SpyContextService['addContextChipCalls'] = [];
	const removeContextChipCalls: SpyContextService['removeContextChipCalls'] = [];
	const clearContextCalls: SpyContextService['clearContextCalls'] = [];

	return {
		_serviceBrand: undefined,
		onDidChangeContext: Event.None,
		addContextChipCalls,
		removeContextChipCalls,
		clearContextCalls,
		getContextChips() { return []; },
		addContextChip(position: PanePosition | undefined, item: ForgeContextItem, automatic?: boolean) {
			addContextChipCalls.push({ position, item, automatic });
		},
		removeContextChip(position: PanePosition | undefined, item: ForgeContextItem) {
			removeContextChipCalls.push({ position, item });
		},
		clearContext(position: PanePosition | undefined) {
			clearContextCalls.push({ position });
		},
		async resolveContextPrompt() {
			return { maxTokens: 0, usedTokens: 0, items: [], droppedCount: 0 };
		},
		async showContextPicker() { return []; },
	};
}

function makeForgeLayoutService(overrides?: Partial<IForgeLayoutService>): IForgeLayoutService {
	return {
		_serviceBrand: undefined,
		activeLayout: 'code+ai' as ForgeLayout,
		onDidChangeLayout: Event.None,
		async setLayout() { },
		async openChatPane() { },
		getLayoutState(): ForgeLayoutState {
			return {
				layout: 'code+ai',
				panes: [
					{ position: 'tr', providerId: 'anthropic', conversationId: 'conv-1' },
				],
			};
		},
		saveLayout() { },
		async restoreLayout() { },
		...overrides,
	};
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

suite('ForgeActiveEditorContextProvider', () => {

	let disposables: DisposableStore;
	let fileService: IFileService;
	let onDidActiveEditorChangeEmitter: Emitter<void>;
	let onDidChangeConfigurationEmitter: Emitter<IConfigurationChangeEvent>;
	let autoAttachEnabled: boolean;
	let activeEditor: EditorInput | undefined;

	setup(() => {
		disposables = new DisposableStore();

		fileService = disposables.add(new FileService(new NullLogService()));
		const fsProvider = disposables.add(new InMemoryFileSystemProvider());
		disposables.add(fileService.registerProvider(Schemas.file, fsProvider));

		onDidActiveEditorChangeEmitter = disposables.add(new Emitter<void>());
		onDidChangeConfigurationEmitter = disposables.add(new Emitter<IConfigurationChangeEvent>());
		autoAttachEnabled = true;
		activeEditor = undefined;
	});

	teardown(() => {
		disposables.dispose();
	});

	ensureNoDisposablesAreLeakedInTestSuite();

	function makeEditorService(): Partial<IEditorService> {
		return {
			onDidActiveEditorChange: onDidActiveEditorChangeEmitter.event,
			get activeEditor() { return activeEditor; },
		};
	}

	function makeConfigService(): { getValue(key: string): unknown; onDidChangeConfiguration: Event<IConfigurationChangeEvent> } {
		return {
			getValue: (key: string) => {
				if (key === 'forge.autoAttachActiveEditor') {
					return autoAttachEnabled;
				}
				return undefined;
			},
			onDidChangeConfiguration: onDidChangeConfigurationEmitter.event,
		};
	}

	async function writeFile(path: string, content: string): Promise<URI> {
		const uri = URI.file(path);
		await fileService.writeFile(uri, VSBuffer.fromString(content));
		return uri;
	}

	function createProvider(overrides?: {
		layoutService?: IForgeLayoutService;
		contextService?: SpyContextService;
	}): { provider: ForgeActiveEditorContextProvider; contextService: SpyContextService } {
		const contextService = overrides?.contextService ?? makeSpyContextService();
		const layoutService = overrides?.layoutService ?? makeForgeLayoutService();

		const provider = new ForgeActiveEditorContextProvider(
			makeEditorService() as IEditorService,
			layoutService,
			contextService as unknown as IForgeContextService,
			fileService,
			makeConfigService() as unknown as import('../../../../../platform/configuration/common/configuration.js').IConfigurationService,
			new NullLogService(),
		);

		disposables.add(provider);
		return { provider, contextService };
	}

	// -----------------------------------------------------------------------
	// Core behaviour
	// -----------------------------------------------------------------------

	test('active code file change adds automatic context chip to nearest AI pane', async () => {
		const uri = await writeFile('/workspace/src/app.ts', 'const app = true;');
		activeEditor = disposables.add(new MockEditorInput(uri));

		const { contextService } = createProvider();

		onDidActiveEditorChangeEmitter.fire();
		// Allow the async updateActiveEditorContext to complete
		await new Promise<void>(r => setTimeout(r, 0));

		assert.strictEqual(contextService.addContextChipCalls.length, 1);
		const call = contextService.addContextChipCalls[0];
		assert.strictEqual(call.position, 'tr');
		assert.strictEqual(call.item.type, ForgeContextType.ActiveEditor);
		assert.strictEqual(call.item.label, 'app.ts');
		assert.strictEqual(call.item.content, 'const app = true;');
	});

	test('switching to a different code file replaces the previous active editor chip', async () => {
		const uriA = await writeFile('/workspace/src/fileA.ts', 'file A content');
		const uriB = await writeFile('/workspace/src/fileB.ts', 'file B content');
		const editorA = disposables.add(new MockEditorInput(uriA));
		const editorB = disposables.add(new MockEditorInput(uriB));

		const { contextService } = createProvider();

		// Switch to file A
		activeEditor = editorA;
		onDidActiveEditorChangeEmitter.fire();
		await new Promise<void>(r => setTimeout(r, 0));

		assert.strictEqual(contextService.addContextChipCalls.length, 1);
		assert.strictEqual(contextService.addContextChipCalls[0].item.label, 'fileA.ts');

		// Switch to file B — should remove A first, then add B
		activeEditor = editorB;
		onDidActiveEditorChangeEmitter.fire();
		await new Promise<void>(r => setTimeout(r, 0));

		assert.strictEqual(contextService.removeContextChipCalls.length, 1);
		assert.strictEqual(contextService.removeContextChipCalls[0].item.label, 'fileA.ts');
		assert.strictEqual(contextService.addContextChipCalls.length, 2);
		assert.strictEqual(contextService.addContextChipCalls[1].item.label, 'fileB.ts');
	});

	test('switching to a ForgeChatInput does not add context chip', async () => {
		const chatInput = disposables.add(new ForgeChatInput('anthropic', 'conv-1'));
		activeEditor = chatInput;

		const { contextService } = createProvider();

		onDidActiveEditorChangeEmitter.fire();
		await new Promise<void>(r => setTimeout(r, 0));

		assert.strictEqual(contextService.addContextChipCalls.length, 0);
	});

	test('switching to untitled editor does not add context chip', async () => {
		// Editor with no resource (untitled)
		activeEditor = disposables.add(new MockEditorInput(undefined));

		const { contextService } = createProvider();

		onDidActiveEditorChangeEmitter.fire();
		await new Promise<void>(r => setTimeout(r, 0));

		assert.strictEqual(contextService.addContextChipCalls.length, 0);
	});

	// -----------------------------------------------------------------------
	// Layout-dependent routing
	// -----------------------------------------------------------------------

	test('in code+ai layout, active editor context goes to tr pane', async () => {
		const uri = await writeFile('/workspace/src/codeai.ts', 'code+ai content');
		activeEditor = disposables.add(new MockEditorInput(uri));

		const layoutService = makeForgeLayoutService({
			activeLayout: 'code+ai',
			getLayoutState: () => ({
				layout: 'code+ai',
				panes: [{ position: 'tr', providerId: 'anthropic', conversationId: 'conv-1' }],
			}),
		});

		const { contextService } = createProvider({ layoutService });

		onDidActiveEditorChangeEmitter.fire();
		await new Promise<void>(r => setTimeout(r, 0));

		assert.strictEqual(contextService.addContextChipCalls.length, 1);
		assert.strictEqual(contextService.addContextChipCalls[0].position, 'tr');
	});

	test('in quad layout, TL code editor context goes to TR pane', async () => {
		const uri = await writeFile('/workspace/src/quad.ts', 'quad content');
		activeEditor = disposables.add(new MockEditorInput(uri));

		const layoutService = makeForgeLayoutService({
			activeLayout: 'quad',
			getLayoutState: () => ({
				layout: 'quad',
				panes: [
					{ position: 'tl', providerId: 'anthropic', conversationId: 'conv-1' },
					{ position: 'tr', providerId: 'openai', conversationId: 'conv-2' },
					{ position: 'bl', providerId: 'anthropic', conversationId: 'conv-3' },
					{ position: 'br', providerId: 'local', conversationId: 'conv-4' },
				],
			}),
		});

		const { contextService } = createProvider({ layoutService });

		onDidActiveEditorChangeEmitter.fire();
		await new Promise<void>(r => setTimeout(r, 0));

		assert.strictEqual(contextService.addContextChipCalls.length, 1);
		assert.strictEqual(contextService.addContextChipCalls[0].position, 'tr');
	});

	test('in quad layout without tr pane, falls back to br pane', async () => {
		const uri = await writeFile('/workspace/src/quad-br.ts', 'quad br content');
		activeEditor = disposables.add(new MockEditorInput(uri));

		const layoutService = makeForgeLayoutService({
			activeLayout: 'quad',
			getLayoutState: () => ({
				layout: 'quad',
				panes: [
					{ position: 'tl', providerId: 'anthropic', conversationId: 'conv-1' },
					{ position: 'bl', providerId: 'anthropic', conversationId: 'conv-3' },
					{ position: 'br', providerId: 'local', conversationId: 'conv-4' },
				],
			}),
		});

		const { contextService } = createProvider({ layoutService });

		onDidActiveEditorChangeEmitter.fire();
		await new Promise<void>(r => setTimeout(r, 0));

		assert.strictEqual(contextService.addContextChipCalls.length, 1);
		assert.strictEqual(contextService.addContextChipCalls[0].position, 'br');
	});

	test('in focus layout, no automatic context added', async () => {
		const uri = await writeFile('/workspace/src/focus.ts', 'focus content');
		activeEditor = disposables.add(new MockEditorInput(uri));

		const layoutService = makeForgeLayoutService({
			activeLayout: 'focus',
			getLayoutState: () => ({
				layout: 'focus',
				panes: [{ position: 'tl', providerId: 'anthropic', conversationId: 'conv-1' }],
			}),
		});

		const { contextService } = createProvider({ layoutService });

		onDidActiveEditorChangeEmitter.fire();
		await new Promise<void>(r => setTimeout(r, 0));

		assert.strictEqual(contextService.addContextChipCalls.length, 0);
	});

	test('in split layout, no automatic context added', async () => {
		const uri = await writeFile('/workspace/src/split.ts', 'split content');
		activeEditor = disposables.add(new MockEditorInput(uri));

		const layoutService = makeForgeLayoutService({
			activeLayout: 'split',
			getLayoutState: () => ({
				layout: 'split',
				panes: [
					{ position: 'tl', providerId: 'anthropic', conversationId: 'conv-1' },
					{ position: 'tr', providerId: 'openai', conversationId: 'conv-2' },
				],
			}),
		});

		const { contextService } = createProvider({ layoutService });

		onDidActiveEditorChangeEmitter.fire();
		await new Promise<void>(r => setTimeout(r, 0));

		assert.strictEqual(contextService.addContextChipCalls.length, 0);
	});

	// -----------------------------------------------------------------------
	// Automatic flag
	// -----------------------------------------------------------------------

	test('context chip is marked as automatic true', async () => {
		const uri = await writeFile('/workspace/src/auto.ts', 'auto content');
		activeEditor = disposables.add(new MockEditorInput(uri));

		const { contextService } = createProvider();

		onDidActiveEditorChangeEmitter.fire();
		await new Promise<void>(r => setTimeout(r, 0));

		assert.strictEqual(contextService.addContextChipCalls.length, 1);
		assert.strictEqual(contextService.addContextChipCalls[0].automatic, true);
	});

	// -----------------------------------------------------------------------
	// Removal ordering
	// -----------------------------------------------------------------------

	test('previous automatic chip is removed before adding new one', async () => {
		const uriA = await writeFile('/workspace/src/first.ts', 'first content');
		const uriB = await writeFile('/workspace/src/second.ts', 'second content');
		const editorA = disposables.add(new MockEditorInput(uriA));
		const editorB = disposables.add(new MockEditorInput(uriB));

		const operations: Array<{ op: 'add' | 'remove'; label: string }> = [];
		const contextService = makeSpyContextService();

		// Instrument the spy to track operation order
		const originalAdd = contextService.addContextChip.bind(contextService);
		const originalRemove = contextService.removeContextChip.bind(contextService);

		contextService.addContextChip = (pos: PanePosition | undefined, item: ForgeContextItem, auto?: boolean) => {
			operations.push({ op: 'add', label: item.label });
			originalAdd(pos, item, auto);
		};
		contextService.removeContextChip = (pos: PanePosition | undefined, item: ForgeContextItem) => {
			operations.push({ op: 'remove', label: item.label });
			originalRemove(pos, item);
		};

		createProvider({ contextService });

		// Fire with file A
		activeEditor = editorA;
		onDidActiveEditorChangeEmitter.fire();
		await new Promise<void>(r => setTimeout(r, 0));

		// Fire with file B
		activeEditor = editorB;
		onDidActiveEditorChangeEmitter.fire();
		await new Promise<void>(r => setTimeout(r, 0));

		// Verify order: add A, remove A, add B
		assert.strictEqual(operations.length, 3);
		assert.deepStrictEqual(operations[0], { op: 'add', label: 'first.ts' });
		assert.deepStrictEqual(operations[1], { op: 'remove', label: 'first.ts' });
		assert.deepStrictEqual(operations[2], { op: 'add', label: 'second.ts' });
	});

	// -----------------------------------------------------------------------
	// Configuration toggle
	// -----------------------------------------------------------------------

	test('disabling auto-attach clears existing automatic context', async () => {
		const uri = await writeFile('/workspace/src/toggle.ts', 'toggle content');
		activeEditor = disposables.add(new MockEditorInput(uri));

		const { contextService } = createProvider();

		// Add automatic context
		onDidActiveEditorChangeEmitter.fire();
		await new Promise<void>(r => setTimeout(r, 0));

		assert.strictEqual(contextService.addContextChipCalls.length, 1);

		// Disable auto-attach
		autoAttachEnabled = false;
		onDidChangeConfigurationEmitter.fire(makeConfigChangeEvent('forge.autoAttachActiveEditor'));
		await new Promise<void>(r => setTimeout(r, 0));

		// Should have removed the previously attached chip
		assert.strictEqual(contextService.removeContextChipCalls.length, 1);
		assert.strictEqual(contextService.removeContextChipCalls[0].item.label, 'toggle.ts');
	});

	test('when auto-attach is disabled, editor changes do not add context', async () => {
		autoAttachEnabled = false;
		const uri = await writeFile('/workspace/src/disabled.ts', 'disabled content');
		activeEditor = disposables.add(new MockEditorInput(uri));

		const { contextService } = createProvider();

		onDidActiveEditorChangeEmitter.fire();
		await new Promise<void>(r => setTimeout(r, 0));

		assert.strictEqual(contextService.addContextChipCalls.length, 0);
	});

	// -----------------------------------------------------------------------
	// Context item shape
	// -----------------------------------------------------------------------

	test('context item has correct type, label, detail, content, and tokenEstimate', async () => {
		const content = 'export function greet() { return "hello"; }';
		const uri = await writeFile('/workspace/src/deep/nested/greet.ts', content);
		activeEditor = disposables.add(new MockEditorInput(uri));

		const { contextService } = createProvider();

		onDidActiveEditorChangeEmitter.fire();
		await new Promise<void>(r => setTimeout(r, 0));

		assert.strictEqual(contextService.addContextChipCalls.length, 1);
		const item = contextService.addContextChipCalls[0].item;
		assert.strictEqual(item.type, ForgeContextType.ActiveEditor);
		assert.strictEqual(item.label, 'greet.ts');
		assert.strictEqual(item.detail, uri.path);
		assert.strictEqual(item.content, content);
		assert.strictEqual(item.tokenEstimate, Math.ceil(content.length / 4));
		assert.deepStrictEqual(item.uri, uri);
	});

	// -----------------------------------------------------------------------
	// Event
	// -----------------------------------------------------------------------

	test('onDidChangeActiveContext fires with item when editor changes', async () => {
		const uri = await writeFile('/workspace/src/event.ts', 'event content');
		activeEditor = disposables.add(new MockEditorInput(uri));

		const { provider } = createProvider();

		const firedItems: Array<ForgeContextItem | undefined> = [];
		disposables.add(provider.onDidChangeActiveContext(item => firedItems.push(item)));

		onDidActiveEditorChangeEmitter.fire();
		await new Promise<void>(r => setTimeout(r, 0));

		assert.strictEqual(firedItems.length, 1);
		assert.strictEqual(firedItems[0]?.label, 'event.ts');
	});

	test('onDidChangeActiveContext fires undefined when context is cleared', async () => {
		const uri = await writeFile('/workspace/src/clear-event.ts', 'clear event content');
		activeEditor = disposables.add(new MockEditorInput(uri));

		const { provider } = createProvider();

		const firedItems: Array<ForgeContextItem | undefined> = [];
		disposables.add(provider.onDidChangeActiveContext(item => firedItems.push(item)));

		// Add context
		onDidActiveEditorChangeEmitter.fire();
		await new Promise<void>(r => setTimeout(r, 0));

		// Clear by switching to no-resource editor
		activeEditor = disposables.add(new MockEditorInput(undefined));
		onDidActiveEditorChangeEmitter.fire();
		await new Promise<void>(r => setTimeout(r, 0));

		assert.strictEqual(firedItems.length, 2);
		assert.strictEqual(firedItems[0]?.label, 'clear-event.ts');
		assert.strictEqual(firedItems[1], undefined);
	});
});
