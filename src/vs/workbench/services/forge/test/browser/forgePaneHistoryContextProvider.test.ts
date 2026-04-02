/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import assert from 'assert';
import { Event } from '../../../../../base/common/event.js';
import { DisposableStore } from '../../../../../base/common/lifecycle.js';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import { NullLogService } from '../../../../../platform/log/common/log.js';
import { ForgeContextType, formatContextItem, type ForgeContextItem } from '../../common/forgeContextTypes.js';
import type { ForgeLayout, ForgeLayoutState, IForgeLayoutService, PanePosition, ForgePaneState } from '../../common/forgeLayoutService.js';
import type { IEditorGroupsService, IEditorGroup, GroupsOrder } from '../../../../services/editor/common/editorGroupsService.js';
import { ForgeChatInput } from '../../../../browser/parts/editor/forgeChat/forgeChatInput.js';
import { ForgePaneHistoryContextProvider } from '../../browser/contextProviders/forgePaneHistoryContextProvider.js';
import { EditorInput } from '../../../../common/editor/editorInput.js';

// ---------------------------------------------------------------------------
// Mock helpers
// ---------------------------------------------------------------------------

function makePaneStates(positions: PanePosition[], providers: string[]): ForgePaneState[] {
	return positions.map((position, i) => ({
		position,
		providerId: providers[i] ?? 'anthropic',
		conversationId: `conv-${i + 1}`,
	}));
}

function makeForgeLayoutService(panes: ForgePaneState[], layout: ForgeLayout = 'quad'): IForgeLayoutService {
	return {
		_serviceBrand: undefined,
		activeLayout: layout,
		onDidChangeLayout: Event.None,
		async setLayout() { },
		async openChatPane() { },
		getLayoutState(): ForgeLayoutState {
			return { layout, panes };
		},
		saveLayout() { },
		async restoreLayout() { },
	};
}

/**
 * Minimal IEditorGroup mock with controllable activeEditor.
 * Only the properties used by ForgePaneHistoryContextProvider are implemented.
 */
function makeMockEditorGroup(id: number, activeEditor: EditorInput | null): Partial<IEditorGroup> {
	return {
		id,
		activeEditor,
		onDidModelChange: Event.None,
		onWillDispose: Event.None,
		onDidActiveEditorChange: Event.None,
		onWillCloseEditor: Event.None,
		onDidCloseEditor: Event.None,
		onWillMoveEditor: Event.None,
	};
}

/**
 * IEditorGroupsService mock. Groups are returned in index order matching
 * the position-to-index mapping in the provider: tl=0, tr=1, bl=2, br=3.
 */
function makeEditorGroupsService(groups: Partial<IEditorGroup>[]): Partial<IEditorGroupsService> {
	return {
		getGroups: (_order?: GroupsOrder) => groups as IEditorGroup[],
		getGroup: (id: number) => (groups.find(g => g.id === id) as IEditorGroup) ?? undefined,
		onDidRemoveGroup: Event.None,
		onDidAddGroup: Event.None,
	};
}

function makeChatInput(provider: string, conversationId: string): ForgeChatInput {
	return new ForgeChatInput(provider, conversationId);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

suite('ForgePaneHistoryContextProvider', () => {

	let disposables: DisposableStore;

	setup(() => {
		disposables = new DisposableStore();
	});

	teardown(() => {
		disposables.dispose();
	});

	ensureNoDisposablesAreLeakedInTestSuite();

	function createProvider(
		panes: ForgePaneState[],
		groups: Partial<IEditorGroup>[],
		layout: ForgeLayout = 'quad',
	): ForgePaneHistoryContextProvider {
		const layoutService = makeForgeLayoutService(panes, layout);
		const editorGroupsService = makeEditorGroupsService(groups);
		const provider = new ForgePaneHistoryContextProvider(
			layoutService as IForgeLayoutService,
			editorGroupsService as IEditorGroupsService,
			new NullLogService(),
		);
		return disposables.add(provider);
	}

	// Standard 4-pane setup
	function makeQuadPanes(): ForgePaneState[] {
		return makePaneStates(
			['tl', 'tr', 'bl', 'br'],
			['anthropic', 'openai', 'anthropic', 'local'],
		);
	}

	function makeQuadGroups(): Partial<IEditorGroup>[] {
		return [
			makeMockEditorGroup(1, disposables.add(makeChatInput('anthropic', 'conv-1'))),
			makeMockEditorGroup(2, disposables.add(makeChatInput('openai', 'conv-2'))),
			makeMockEditorGroup(3, disposables.add(makeChatInput('anthropic', 'conv-3'))),
			makeMockEditorGroup(4, disposables.add(makeChatInput('local', 'conv-4'))),
		];
	}

	// -----------------------------------------------------------------------
	// getAvailablePaneHistories
	// -----------------------------------------------------------------------

	suite('getAvailablePaneHistories', () => {

		test('excludes the requesting pane', () => {
			const provider = createProvider(makeQuadPanes(), makeQuadGroups());

			const items = provider.getAvailablePaneHistories('tl');

			assert.strictEqual(items.length, 3);
			const positions = items.map(item => item.sourcePanePosition);
			assert.ok(!positions.includes('tl'), 'Should not include the requesting pane');
			assert.ok(positions.includes('tr'));
			assert.ok(positions.includes('bl'));
			assert.ok(positions.includes('br'));
		});

		test('returns empty array when only one pane exists', () => {
			const panes = makePaneStates(['tl'], ['anthropic']);
			const groups = [makeMockEditorGroup(1, disposables.add(makeChatInput('anthropic', 'conv-1')))];

			const provider = createProvider(panes, groups, 'focus');

			const items = provider.getAvailablePaneHistories('tl');

			assert.ok(Array.isArray(items));
			assert.strictEqual(items.length, 0);
		});

		test('returns items for all other active panes', () => {
			const panes = makePaneStates(['tl', 'tr', 'bl'], ['anthropic', 'openai', 'anthropic']);
			const groups = [
				makeMockEditorGroup(1, disposables.add(makeChatInput('anthropic', 'conv-1'))),
				makeMockEditorGroup(2, disposables.add(makeChatInput('openai', 'conv-2'))),
				makeMockEditorGroup(3, disposables.add(makeChatInput('anthropic', 'conv-3'))),
			];

			const provider = createProvider(panes, groups, 'quad');

			const items = provider.getAvailablePaneHistories('tl');

			assert.strictEqual(items.length, 2);
			const positions = items.map(item => item.sourcePanePosition);
			assert.ok(positions.includes('tr'));
			assert.ok(positions.includes('bl'));
		});

		test('each item label includes pane position and provider name', () => {
			const provider = createProvider(makeQuadPanes(), makeQuadGroups());

			const items = provider.getAvailablePaneHistories('tl');

			// Label format: "Pane TR \u2014 openai"
			const trItem = items.find(i => i.sourcePanePosition === 'tr');
			assert.ok(trItem, 'Should have a TR item');
			assert.strictEqual(trItem!.label, 'Pane TR \u2014 openai');

			const blItem = items.find(i => i.sourcePanePosition === 'bl');
			assert.ok(blItem, 'Should have a BL item');
			assert.strictEqual(blItem!.label, 'Pane BL \u2014 anthropic');

			const brItem = items.find(i => i.sourcePanePosition === 'br');
			assert.ok(brItem, 'Should have a BR item');
			assert.strictEqual(brItem!.label, 'Pane BR \u2014 local');
		});

		test('items have type PaneHistory', () => {
			const provider = createProvider(makeQuadPanes(), makeQuadGroups());

			const items = provider.getAvailablePaneHistories('tl');

			for (const item of items) {
				assert.strictEqual(item.type, ForgeContextType.PaneHistory);
			}
		});

		test('items include sourcePanePosition', () => {
			const provider = createProvider(makeQuadPanes(), makeQuadGroups());

			const items = provider.getAvailablePaneHistories('bl');

			for (const item of items) {
				assert.ok(item.sourcePanePosition, `Item "${item.label}" should have sourcePanePosition`);
			}

			const positions = items.map(i => i.sourcePanePosition);
			assert.ok(positions.includes('tl'));
			assert.ok(positions.includes('tr'));
			assert.ok(positions.includes('br'));
			assert.ok(!positions.includes('bl'), 'Should not include the requesting pane');
		});

		test('items have empty content and zero tokenEstimate (lazy resolution)', () => {
			const provider = createProvider(makeQuadPanes(), makeQuadGroups());

			const items = provider.getAvailablePaneHistories('tl');

			for (const item of items) {
				assert.strictEqual(item.content, '');
				assert.strictEqual(item.tokenEstimate, 0);
			}
		});

		test('items include detail with pane label', () => {
			const provider = createProvider(makeQuadPanes(), makeQuadGroups());

			const items = provider.getAvailablePaneHistories('tl');

			for (const item of items) {
				assert.ok(item.detail, 'Each item should have a detail string');
				assert.ok(
					item.detail!.includes('Conversation history from'),
					`Detail "${item.detail}" should describe conversation history`,
				);
			}
		});

		test('returns empty array when excludePosition is undefined and layout has no panes', () => {
			const provider = createProvider([], [], 'focus');

			const items = provider.getAvailablePaneHistories(undefined);

			assert.ok(Array.isArray(items));
			assert.strictEqual(items.length, 0);
		});

		test('skips panes without a ForgeChatInput active editor', () => {
			const panes = makePaneStates(['tl', 'tr', 'bl'], ['anthropic', 'openai', 'anthropic']);
			// Group at index 1 (tr) has null activeEditor
			const groups = [
				makeMockEditorGroup(1, disposables.add(makeChatInput('anthropic', 'conv-1'))),
				makeMockEditorGroup(2, null),
				makeMockEditorGroup(3, disposables.add(makeChatInput('anthropic', 'conv-3'))),
			];

			const provider = createProvider(panes, groups, 'quad');

			const items = provider.getAvailablePaneHistories('tl');

			// Only bl should be returned since tr has no chat editor
			const positions = items.map(i => i.sourcePanePosition);
			assert.ok(positions.includes('bl'));
			assert.ok(!positions.includes('tr'), 'Should not include pane without chat editor');
			assert.ok(!positions.includes('tl'), 'Should not include requesting pane');
		});

		test('skips panes whose group index exceeds available groups', () => {
			// Panes declare all 4 positions but only 2 groups exist
			const panes = makeQuadPanes();
			const groups = [
				makeMockEditorGroup(1, disposables.add(makeChatInput('anthropic', 'conv-1'))),
				makeMockEditorGroup(2, disposables.add(makeChatInput('openai', 'conv-2'))),
			];

			const provider = createProvider(panes, groups, 'quad');

			const items = provider.getAvailablePaneHistories('tl');

			// bl (index 2) and br (index 3) are out of bounds, so only tr should appear
			const positions = items.map(i => i.sourcePanePosition);
			assert.ok(positions.includes('tr'));
			assert.ok(!positions.includes('bl'), 'Should not include pane with no matching group');
			assert.ok(!positions.includes('br'), 'Should not include pane with no matching group');
		});
	});

	// -----------------------------------------------------------------------
	// resolvePaneHistory
	// -----------------------------------------------------------------------

	suite('resolvePaneHistory', () => {

		test('returns item with provider info for valid pane', () => {
			const provider = createProvider(makeQuadPanes(), makeQuadGroups());

			const item = provider.resolvePaneHistory('tr');

			assert.ok(item, 'Should return an item');
			assert.strictEqual(item.type, ForgeContextType.PaneHistory);
			assert.strictEqual(item.label, 'Pane TR \u2014 openai');
		});

		test('returns unavailable item for missing pane', () => {
			const panes = makePaneStates(['tl'], ['anthropic']);
			const groups = [makeMockEditorGroup(1, disposables.add(makeChatInput('anthropic', 'conv-1')))];

			const provider = createProvider(panes, groups, 'focus');

			const item = provider.resolvePaneHistory('br');

			assert.ok(item, 'Should return an item even for missing pane');
			assert.strictEqual(item.type, ForgeContextType.PaneHistory);
			assert.strictEqual(item.label, 'Pane BR \u2014 unavailable');
			assert.strictEqual(item.content, 'Pane no longer available');
			assert.strictEqual(item.sourcePanePosition, 'br');
		});

		test('returns correct type and sourcePanePosition', () => {
			const provider = createProvider(makeQuadPanes(), makeQuadGroups());

			const item = provider.resolvePaneHistory('bl');

			assert.strictEqual(item.type, ForgeContextType.PaneHistory);
			assert.strictEqual(item.sourcePanePosition, 'bl');
		});

		test('token estimate is zero for lazily-resolved content', () => {
			const provider = createProvider(makeQuadPanes(), makeQuadGroups());

			const item = provider.resolvePaneHistory('tr');

			// Content is empty (resolved later by ForgeChatView), so tokenEstimate is 0
			assert.strictEqual(item.content, '');
			assert.strictEqual(item.tokenEstimate, 0);
		});

		test('unavailable pane has zero tokenEstimate', () => {
			const panes = makePaneStates(['tl'], ['anthropic']);
			const groups = [makeMockEditorGroup(1, disposables.add(makeChatInput('anthropic', 'conv-1')))];

			const provider = createProvider(panes, groups, 'focus');

			const item = provider.resolvePaneHistory('br');

			assert.strictEqual(item.tokenEstimate, 0);
		});

		test('sourcePanePosition matches the requested position for all positions', () => {
			const provider = createProvider(makeQuadPanes(), makeQuadGroups());

			for (const pos of ['tl', 'tr', 'bl', 'br'] as PanePosition[]) {
				const item = provider.resolvePaneHistory(pos);
				assert.strictEqual(item.sourcePanePosition, pos, `sourcePanePosition should be "${pos}"`);
			}
		});

		test('valid pane includes detail about conversation history', () => {
			const provider = createProvider(makeQuadPanes(), makeQuadGroups());

			const item = provider.resolvePaneHistory('tr');

			assert.ok(item.detail, 'Should have detail');
			assert.ok(
				item.detail!.includes('Conversation history from TR pane'),
				`Detail "${item.detail}" should describe TR conversation history`,
			);
		});

		test('label format matches "Pane {POSITION} — {provider}"', () => {
			const provider = createProvider(makeQuadPanes(), makeQuadGroups());

			const tlItem = provider.resolvePaneHistory('tl');
			assert.strictEqual(tlItem.label, 'Pane TL \u2014 anthropic');

			const trItem = provider.resolvePaneHistory('tr');
			assert.strictEqual(trItem.label, 'Pane TR \u2014 openai');

			const blItem = provider.resolvePaneHistory('bl');
			assert.strictEqual(blItem.label, 'Pane BL \u2014 anthropic');

			const brItem = provider.resolvePaneHistory('br');
			assert.strictEqual(brItem.label, 'Pane BR \u2014 local');
		});
	});

	// -----------------------------------------------------------------------
	// Cross-pane context integration
	// -----------------------------------------------------------------------

	suite('cross-pane context integration', () => {

		test('context chip from getAvailablePaneHistories has correct sourcePanePosition', () => {
			const provider = createProvider(makeQuadPanes(), makeQuadGroups());

			const items = provider.getAvailablePaneHistories('tl');

			for (const item of items) {
				assert.ok(item.sourcePanePosition, 'Each item should have sourcePanePosition');
				assert.ok(
					['tr', 'bl', 'br'].includes(item.sourcePanePosition!),
					`sourcePanePosition "${item.sourcePanePosition}" should be one of the non-requesting panes`,
				);
			}
		});

		test('context prompt formats pane history as <context> XML block', () => {
			const item: ForgeContextItem = {
				type: ForgeContextType.PaneHistory,
				label: 'Pane TR \u2014 openai',
				content: 'User: hello\nAssistant: hi there',
				tokenEstimate: Math.ceil('User: hello\nAssistant: hi there'.length / 4),
				sourcePanePosition: 'tr',
			};

			const formatted = formatContextItem(item);

			// formatContextItem uses: `<context pane="${item.label}">${item.content}</context>`
			assert.strictEqual(
				formatted,
				'<context pane="Pane TR \u2014 openai">User: hello\nAssistant: hi there</context>',
			);
		});

		test('formatContextItem uses item label as pane attribute value', () => {
			const item: ForgeContextItem = {
				type: ForgeContextType.PaneHistory,
				label: 'Pane BL \u2014 anthropic',
				content: 'conversation text',
				tokenEstimate: 5,
				sourcePanePosition: 'bl',
			};

			const formatted = formatContextItem(item);

			assert.ok(
				formatted.includes('pane="Pane BL \u2014 anthropic"'),
				`Should use label as pane attribute, got: "${formatted}"`,
			);
		});

		test('resolved item from resolvePaneHistory can be formatted as context', () => {
			const provider = createProvider(makeQuadPanes(), makeQuadGroups());

			const item = provider.resolvePaneHistory('tr');
			const formatted = formatContextItem(item);

			assert.ok(typeof formatted === 'string');
			assert.ok(formatted.startsWith('<context'));
			assert.ok(formatted.endsWith('</context>'));
		});

		test('items from getAvailablePaneHistories have consistent structure', () => {
			const provider = createProvider(makeQuadPanes(), makeQuadGroups());

			const items = provider.getAvailablePaneHistories('tl');

			for (const item of items) {
				assert.strictEqual(item.type, ForgeContextType.PaneHistory);
				assert.strictEqual(typeof item.label, 'string');
				assert.ok(item.label.length > 0, 'label should not be empty');
				assert.strictEqual(typeof item.content, 'string');
				assert.strictEqual(typeof item.tokenEstimate, 'number');
				assert.ok(item.tokenEstimate >= 0, 'tokenEstimate should be non-negative');
				assert.ok(item.sourcePanePosition, 'sourcePanePosition should be defined');
			}
		});

		test('getAvailablePaneHistories and resolvePaneHistory return same label for same position', () => {
			const provider = createProvider(makeQuadPanes(), makeQuadGroups());

			const items = provider.getAvailablePaneHistories('tl');
			const trAvailable = items.find(i => i.sourcePanePosition === 'tr');
			const trResolved = provider.resolvePaneHistory('tr');

			assert.ok(trAvailable, 'Should find TR in available items');
			assert.strictEqual(trAvailable!.label, trResolved.label);
		});
	});
});
