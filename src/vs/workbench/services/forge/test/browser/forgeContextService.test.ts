/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import assert from 'assert';
import { Emitter, Event } from '../../../../../base/common/event.js';
import { DisposableStore } from '../../../../../base/common/lifecycle.js';
import { URI } from '../../../../../base/common/uri.js';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import { NullLogService } from '../../../../../platform/log/common/log.js';
import type { IInstantiationService } from '../../../../../platform/instantiation/common/instantiation.js';
import type { IQuickInputService, IQuickPickItem } from '../../../../../platform/quickinput/common/quickInput.js';
import type { IEditorGroupsService } from '../../../../services/editor/common/editorGroupsService.js';
import type { IEditorService } from '../../../../services/editor/common/editorService.js';
import { ForgeContextType, type ForgeContextItem } from '../../common/forgeContextTypes.js';
import { ForgeContextService } from '../../browser/forgeContextService.js';
import type { IForgeContextService } from '../../common/forgeContextService.js';
import type { IForgeGitDiffService } from '../../common/forgeGitDiffService.js';
import type { IForgeLayoutService, ForgeLayout, ForgeLayoutState, PanePosition } from '../../common/forgeLayoutService.js';
import type { IWorkspaceContextService } from '../../../../../platform/workspace/common/workspace.js';

// ---------------------------------------------------------------------------
// Mock helpers
// ---------------------------------------------------------------------------

function makeEditorGroupsService(): Partial<IEditorGroupsService> {
	return {
		onDidRemoveGroup: Event.None,
		onDidAddGroup: Event.None,
		getGroups: () => [],
	};
}

function makeForgeLayoutService(overrides?: Partial<IForgeLayoutService>): IForgeLayoutService {
	return {
		_serviceBrand: undefined,
		activeLayout: 'quad' as ForgeLayout,
		onDidChangeLayout: Event.None,
		async setLayout() { },
		async openChatPane() { },
		getLayoutState(): ForgeLayoutState {
			return {
				layout: 'quad',
				panes: [
					{ position: 'tl', providerId: 'anthropic', conversationId: 'conv-1' },
					{ position: 'tr', providerId: 'openai', conversationId: 'conv-2' },
					{ position: 'bl', providerId: 'anthropic', conversationId: 'conv-3' },
					{ position: 'br', providerId: 'local', conversationId: 'conv-4' },
				],
			};
		},
		saveLayout() { },
		async restoreLayout() { },
		...overrides,
	};
}

function makeEditorService(): Partial<IEditorService> {
	return {
		activeTextEditorControl: undefined,
		activeEditor: undefined,
		editors: [],
	};
}

interface MockQuickPick {
	items: IQuickPickItem[];
	selectedItems: IQuickPickItem[];
	readonly onDidAccept: Event<void>;
	readonly onDidHide: Event<void>;
	show(): void;
	hide(): void;
	dispose(): void;
}

/**
 * Minimal instantiation service mock that returns stub context providers.
 * The stubs implement the methods that ForgeContextService calls on each
 * provider without requiring real service dependencies.
 */
function makeInstantiationService(): Partial<IInstantiationService> {
	return {
		createInstance(_ctorOrDescriptor: unknown, ..._args: unknown[]): unknown {
			const store = new DisposableStore();
			const stub: Record<string, unknown> = {
				dispose: () => store.dispose(),
				// ForgeFileContextProvider stub
				resolveFile: () => Promise.resolve({
					type: ForgeContextType.File,
					label: 'stub',
					content: '',
					tokenEstimate: 0,
				}),
				// ForgePaneHistoryContextProvider stub
				getAvailablePaneHistories: () => [],
				resolvePaneHistory: () => ({
					type: ForgeContextType.PaneHistory,
					label: 'stub',
					content: '',
					tokenEstimate: 0,
				}),
			};
			return stub;
		},
	};
}

function makeQuickInputService(acceptItems?: IQuickPickItem[]): { service: Partial<IQuickInputService>; picker: MockQuickPick } {
	const onDidAcceptEmitter = new Emitter<void>();
	const onDidHideEmitter = new Emitter<void>();

	const picker: MockQuickPick = {
		items: [],
		selectedItems: acceptItems ?? [],
		onDidAccept: onDidAcceptEmitter.event,
		onDidHide: onDidHideEmitter.event,
		show() {
			// Simulate immediate acceptance if items provided
			if (acceptItems) {
				queueMicrotask(() => onDidAcceptEmitter.fire());
			} else {
				// Simulate cancel
				queueMicrotask(() => onDidHideEmitter.fire());
			}
		},
		hide() { onDidHideEmitter.fire(); },
		dispose() {
			onDidAcceptEmitter.dispose();
			onDidHideEmitter.dispose();
		},
	};

	const service: Partial<IQuickInputService> = {
		pick: ((_picks: unknown, _options?: unknown) => {
			return Promise.resolve(acceptItems ?? undefined);
		}) as IQuickInputService['pick'],
	};

	return { service, picker };
}

function makeGitDiffService(): IForgeGitDiffService {
	return {
		_serviceBrand: undefined,
		resolveGitDiff: async () => ({
			type: ForgeContextType.GitDiff,
			label: 'Git Diff (HEAD)',
			content: 'mock diff',
			tokenEstimate: 10,
		}),
	};
}

function makeWorkspaceContextService(): Partial<IWorkspaceContextService> {
	return {
		getWorkspace: () => ({
			id: 'test-workspace',
			folders: [],
			transient: false,
			configuration: undefined,
		}),
	};
}

// ---------------------------------------------------------------------------
// Context item factories
// ---------------------------------------------------------------------------

function makeFileItem(name: string, content: string): ForgeContextItem {
	return {
		type: ForgeContextType.File,
		label: name,
		content,
		tokenEstimate: Math.ceil(content.length / 4),
		uri: URI.file(`/test-workspace/${name}`),
	};
}

function makeSelectionItem(file: string, lines: string, content: string): ForgeContextItem {
	return {
		type: ForgeContextType.Selection,
		label: `Selection in ${file}`,
		detail: lines,
		content,
		tokenEstimate: Math.ceil(content.length / 4),
		uri: URI.file(`/test-workspace/${file}`),
	};
}

function makeDiffItem(content: string): ForgeContextItem {
	return {
		type: ForgeContextType.GitDiff,
		label: 'Git Diff',
		content,
		tokenEstimate: Math.ceil(content.length / 4),
	};
}

function makeSymbolItem(name: string, content: string): ForgeContextItem {
	return {
		type: ForgeContextType.Symbol,
		label: name,
		content,
		tokenEstimate: Math.ceil(content.length / 4),
		uri: URI.file('/test-workspace/symbols.ts'),
	};
}

function makePaneHistoryItem(pane: PanePosition, content: string): ForgeContextItem {
	return {
		type: ForgeContextType.PaneHistory,
		label: `Pane ${pane}`,
		content,
		tokenEstimate: Math.ceil(content.length / 4),
		sourcePanePosition: pane,
	};
}

function makeActiveEditorItem(file: string, content: string): ForgeContextItem {
	return {
		type: ForgeContextType.ActiveEditor,
		label: file,
		content,
		tokenEstimate: Math.ceil(content.length / 4),
		uri: URI.file(`/test-workspace/${file}`),
	};
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

suite('ForgeContextService', () => {

	let disposables: DisposableStore;

	setup(() => {
		disposables = new DisposableStore();
	});

	teardown(() => {
		disposables.dispose();
	});

	ensureNoDisposablesAreLeakedInTestSuite();

	function createService(overrides?: {
		quickInputService?: Partial<IQuickInputService>;
		layoutService?: IForgeLayoutService;
		editorService?: Partial<IEditorService>;
		editorGroupsService?: Partial<IEditorGroupsService>;
		gitDiffService?: IForgeGitDiffService;
		workspaceContextService?: Partial<IWorkspaceContextService>;
	}): IForgeContextService {
		const service = new ForgeContextService(
			(overrides?.editorService ?? makeEditorService()) as IEditorService,
			(overrides?.editorGroupsService ?? makeEditorGroupsService()) as IEditorGroupsService,
			(overrides?.quickInputService ?? makeQuickInputService().service) as IQuickInputService,
			overrides?.layoutService ?? makeForgeLayoutService(),
			new NullLogService(),
			makeInstantiationService() as IInstantiationService,
			overrides?.gitDiffService ?? makeGitDiffService(),
			(overrides?.workspaceContextService ?? makeWorkspaceContextService()) as IWorkspaceContextService,
		);
		return disposables.add(service);
	}

	// -----------------------------------------------------------------------
	// Context chip management
	// -----------------------------------------------------------------------

	suite('context chip management', () => {

		test('addContextChip adds item and fires onDidChangeContext', async () => {
			const service = createService();
			const item = makeFileItem('foo.ts', 'const foo = 1;');

			const eventPromise = Event.toPromise(service.onDidChangeContext);
			service.addContextChip('tl', item);

			const firedPosition = await eventPromise;
			assert.strictEqual(firedPosition, 'tl');

			const chips = service.getContextChips('tl');
			assert.strictEqual(chips.length, 1);
			assert.strictEqual(chips[0].item.label, 'foo.ts');
			assert.strictEqual(chips[0].automatic, false);
		});

		test('addContextChip with automatic flag sets chip.automatic to true', () => {
			const service = createService();
			const item = makeActiveEditorItem('index.ts', 'export default {};');

			service.addContextChip('tl', item, true);

			const chips = service.getContextChips('tl');
			assert.strictEqual(chips.length, 1);
			assert.strictEqual(chips[0].automatic, true);
		});

		test('removeContextChip removes item and fires onDidChangeContext', async () => {
			const service = createService();
			const item = makeFileItem('bar.ts', 'const bar = 2;');

			service.addContextChip('tr', item);
			assert.strictEqual(service.getContextChips('tr').length, 1);

			const eventPromise = Event.toPromise(service.onDidChangeContext);
			service.removeContextChip('tr', item);

			const firedPosition = await eventPromise;
			assert.strictEqual(firedPosition, 'tr');
			assert.strictEqual(service.getContextChips('tr').length, 0);
		});

		test('clearContext removes all chips for a pane', async () => {
			const service = createService();

			service.addContextChip('bl', makeFileItem('a.ts', 'a'));
			service.addContextChip('bl', makeFileItem('b.ts', 'b'));
			service.addContextChip('bl', makeFileItem('c.ts', 'c'));
			assert.strictEqual(service.getContextChips('bl').length, 3);

			const eventPromise = Event.toPromise(service.onDidChangeContext);
			service.clearContext('bl');

			const firedPosition = await eventPromise;
			assert.strictEqual(firedPosition, 'bl');
			assert.strictEqual(service.getContextChips('bl').length, 0);
		});

		test('getContextChips returns empty array for pane with no context', () => {
			const service = createService();
			const chips = service.getContextChips('br');
			assert.ok(Array.isArray(chips));
			assert.strictEqual(chips.length, 0);
		});

		test('context chips are scoped per pane position', () => {
			const service = createService();

			service.addContextChip('tl', makeFileItem('tl-file.ts', 'tl content'));
			service.addContextChip('tr', makeFileItem('tr-file.ts', 'tr content'));
			service.addContextChip('bl', makeFileItem('bl-file.ts', 'bl content'));

			assert.strictEqual(service.getContextChips('tl').length, 1);
			assert.strictEqual(service.getContextChips('tr').length, 1);
			assert.strictEqual(service.getContextChips('bl').length, 1);
			assert.strictEqual(service.getContextChips('br').length, 0);

			assert.strictEqual(service.getContextChips('tl')[0].item.label, 'tl-file.ts');
			assert.strictEqual(service.getContextChips('tr')[0].item.label, 'tr-file.ts');
			assert.strictEqual(service.getContextChips('bl')[0].item.label, 'bl-file.ts');
		});

		test('context chips work with undefined pane position (focus mode)', () => {
			const service = createService();
			const item = makeFileItem('focus.ts', 'focus content');

			service.addContextChip(undefined, item);

			const chips = service.getContextChips(undefined);
			assert.strictEqual(chips.length, 1);
			assert.strictEqual(chips[0].item.label, 'focus.ts');
		});
	});

	// -----------------------------------------------------------------------
	// resolveContextPrompt
	// -----------------------------------------------------------------------

	suite('resolveContextPrompt', () => {

		test('resolveContextPrompt returns all chips when under budget', async () => {
			const service = createService();
			const item1 = makeFileItem('a.ts', 'short');
			const item2 = makeFileItem('b.ts', 'also short');

			service.addContextChip('tl', item1);
			service.addContextChip('tl', item2);

			const budget = await service.resolveContextPrompt('tl', 10000);

			assert.strictEqual(budget.items.length, 2);
			assert.strictEqual(budget.droppedCount, 0);
			assert.ok(budget.usedTokens <= budget.maxTokens);
		});

		test('resolveContextPrompt drops lowest-priority items when over budget', async () => {
			const service = createService();

			// ActiveEditor has highest priority, PaneHistory has lowest
			const highPriority = makeActiveEditorItem('active.ts', 'x'.repeat(80)); // ~20 tokens
			const lowPriority = makePaneHistoryItem('tr', 'y'.repeat(80)); // ~20 tokens

			service.addContextChip('tl', highPriority);
			service.addContextChip('tl', lowPriority);

			// Budget only allows ~25 tokens — enough for one item but not both
			const budget = await service.resolveContextPrompt('tl', 25);

			assert.strictEqual(budget.droppedCount, 1);
			assert.strictEqual(budget.items.length, 1);
			// The kept item should be the higher-priority activeEditor
			assert.strictEqual(budget.items[0].item.type, ForgeContextType.ActiveEditor);
		});

		test('resolveContextPrompt formats file items as <file> XML blocks', async () => {
			const service = createService();
			// Content must be >= 100 chars to skip resolveChipContent (which replaces with stub)
			const item = makeFileItem('utils.ts', 'export function add(a: number, b: number) { return a + b; }' + ' '.repeat(50));

			service.addContextChip('tl', item);

			const budget = await service.resolveContextPrompt('tl', 10000);

			assert.strictEqual(budget.items.length, 1);
			// The item should be formatted — verify the content references file path
			const resolved = budget.items[0];
			assert.strictEqual(resolved.item.type, ForgeContextType.File);
			assert.strictEqual(resolved.item.label, 'utils.ts');
		});

		test('resolveContextPrompt formats selection items as <selection> XML blocks', async () => {
			const service = createService();
			const item = makeSelectionItem('main.ts', '10-20', 'const selected = true;');

			service.addContextChip('tl', item);

			const budget = await service.resolveContextPrompt('tl', 10000);

			assert.strictEqual(budget.items.length, 1);
			assert.strictEqual(budget.items[0].item.type, ForgeContextType.Selection);
		});

		test('resolveContextPrompt formats diff items as <diff> XML blocks', async () => {
			const service = createService();
			const item = makeDiffItem('- old line\n+ new line');

			service.addContextChip('tl', item);

			const budget = await service.resolveContextPrompt('tl', 10000);

			assert.strictEqual(budget.items.length, 1);
			assert.strictEqual(budget.items[0].item.type, ForgeContextType.GitDiff);
		});

		test('resolveContextPrompt formats pane history as <context> XML blocks', async () => {
			const service = createService();
			const item = makePaneHistoryItem('tr', 'User: hello\nAssistant: hi there');

			service.addContextChip('tl', item);

			const budget = await service.resolveContextPrompt('tl', 10000);

			assert.strictEqual(budget.items.length, 1);
			assert.strictEqual(budget.items[0].item.type, ForgeContextType.PaneHistory);
			assert.strictEqual(budget.items[0].item.sourcePanePosition, 'tr');
		});

		test('resolveContextPrompt returns correct usedTokens and droppedCount', async () => {
			const service = createService();

			// Each item is ~25 raw tokens (100 chars / 4) but formatContextItem wraps
			// in XML tags adding ~26 chars, so actual cost is ~32 tokens per item.
			service.addContextChip('tl', makeFileItem('a.ts', 'x'.repeat(100)));
			service.addContextChip('tl', makeFileItem('b.ts', 'y'.repeat(100)));
			service.addContextChip('tl', makeFileItem('c.ts', 'z'.repeat(100)));

			// Budget allows 2 items (~64 tokens) but not 3 (~96 tokens)
			const budget = await service.resolveContextPrompt('tl', 70);

			assert.strictEqual(budget.maxTokens, 70);
			assert.strictEqual(budget.items.length, 2);
			assert.strictEqual(budget.droppedCount, 1);
			assert.ok(budget.usedTokens <= 70, `usedTokens ${budget.usedTokens} should be <= 70`);
			assert.ok(budget.usedTokens > 0, 'usedTokens should be > 0');
		});

		test('resolveContextPrompt with zero maxTokens returns empty budget', async () => {
			const service = createService();
			service.addContextChip('tl', makeFileItem('a.ts', 'content'));

			const budget = await service.resolveContextPrompt('tl', 0);

			assert.strictEqual(budget.maxTokens, 0);
			assert.strictEqual(budget.usedTokens, 0);
			assert.strictEqual(budget.items.length, 0);
			assert.strictEqual(budget.droppedCount, 1);
		});

		test('priority order: activeEditor > selection > file > gitDiff > symbol > paneHistory', async () => {
			const service = createService();

			// Add items in reverse priority order
			const paneHistory = makePaneHistoryItem('tr', 'ph');
			const symbol = makeSymbolItem('myFunc', 'sym');
			const gitDiff = makeDiffItem('diff');
			const file = makeFileItem('f.ts', 'file');
			const selection = makeSelectionItem('s.ts', '1-2', 'sel');
			const activeEditor = makeActiveEditorItem('ae.ts', 'active');

			service.addContextChip('tl', paneHistory);
			service.addContextChip('tl', symbol);
			service.addContextChip('tl', gitDiff);
			service.addContextChip('tl', file);
			service.addContextChip('tl', selection);
			service.addContextChip('tl', activeEditor);

			// Large budget — all items should be included, sorted by priority
			const budget = await service.resolveContextPrompt('tl', 100000);

			assert.strictEqual(budget.items.length, 6);
			assert.strictEqual(budget.droppedCount, 0);

			// Verify priority ordering
			assert.strictEqual(budget.items[0].item.type, ForgeContextType.ActiveEditor);
			assert.strictEqual(budget.items[1].item.type, ForgeContextType.Selection);
			assert.strictEqual(budget.items[2].item.type, ForgeContextType.File);
			assert.strictEqual(budget.items[3].item.type, ForgeContextType.GitDiff);
			assert.strictEqual(budget.items[4].item.type, ForgeContextType.Symbol);
			assert.strictEqual(budget.items[5].item.type, ForgeContextType.PaneHistory);
		});

		test('resolveContextPrompt for pane with no chips returns empty budget', async () => {
			const service = createService();

			const budget = await service.resolveContextPrompt('br', 10000);

			assert.strictEqual(budget.maxTokens, 10000);
			assert.strictEqual(budget.usedTokens, 0);
			assert.strictEqual(budget.items.length, 0);
			assert.strictEqual(budget.droppedCount, 0);
		});
	});

	// -----------------------------------------------------------------------
	// Token estimation
	// -----------------------------------------------------------------------

	suite('token estimation', () => {

		test('empty content estimates 0 tokens', () => {
			const item = makeFileItem('empty.ts', '');
			assert.strictEqual(item.tokenEstimate, 0);
		});

		test('100-character content estimates ~25 tokens', () => {
			const content = 'a'.repeat(100);
			const item = makeFileItem('hundred.ts', content);
			assert.strictEqual(item.tokenEstimate, 25);
		});

		test('4000-character content estimates ~1000 tokens', () => {
			const content = 'b'.repeat(4000);
			const item = makeFileItem('four-thousand.ts', content);
			assert.strictEqual(item.tokenEstimate, 1000);
		});

		test('odd-length content rounds up', () => {
			const content = 'c'.repeat(101); // 101 / 4 = 25.25 → ceil → 26
			const item = makeFileItem('odd.ts', content);
			assert.strictEqual(item.tokenEstimate, Math.ceil(101 / 4));
			assert.strictEqual(item.tokenEstimate, 26);
		});

		test('single character estimates 1 token', () => {
			const item = makeFileItem('single.ts', 'x');
			assert.strictEqual(item.tokenEstimate, 1);
		});
	});

	// -----------------------------------------------------------------------
	// showContextPicker
	// -----------------------------------------------------------------------

	suite('showContextPicker', () => {

		test('showContextPicker calls quickInputService.pick', async () => {
			let pickCalled = false;
			const { service: quickInputService, picker } = makeQuickInputService();
			quickInputService.pick = ((_picks: unknown, _options?: unknown) => {
				pickCalled = true;
				return Promise.resolve(undefined);
			}) as IQuickInputService['pick'];

			const service = createService({ quickInputService });

			// Picker will return undefined (cancel)
			await service.showContextPicker('tl');

			assert.ok(pickCalled, 'quickInputService.pick should have been called');

			picker.dispose();
		});

		test('selected items are returned as ForgeContextItem array', async () => {
			const selectedQuickPickItems: IQuickPickItem[] = [
				{ label: 'test-file.ts', description: '/test-workspace/test-file.ts' },
			];

			const { service: quickInputService, picker } = makeQuickInputService(selectedQuickPickItems);
			const service = createService({ quickInputService });

			const result = await service.showContextPicker('tl');

			assert.ok(Array.isArray(result));
			// Result should contain ForgeContextItem objects
			if (result.length > 0) {
				assert.ok(result[0].type, 'Result items should have a type');
				assert.ok(result[0].label, 'Result items should have a label');
			}

			picker.dispose();
		});

		test('cancelled picker returns empty array', async () => {
			// No accept items — picker will auto-hide (cancel)
			const { service: quickInputService, picker } = makeQuickInputService();
			const service = createService({ quickInputService });

			const result = await service.showContextPicker('tl');

			assert.ok(Array.isArray(result));
			assert.strictEqual(result.length, 0);

			picker.dispose();
		});

		test('showContextPicker works with undefined position (focus mode)', async () => {
			const { service: quickInputService, picker } = makeQuickInputService();
			const service = createService({ quickInputService });

			const result = await service.showContextPicker(undefined);

			assert.ok(Array.isArray(result));
			assert.strictEqual(result.length, 0);

			picker.dispose();
		});
	});
});
