/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import assert from 'assert';
import { Event } from '../../../../../base/common/event.js';
import { DisposableStore } from '../../../../../base/common/lifecycle.js';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import { NullLogService } from '../../../../../platform/log/common/log.js';
import { StorageScope } from '../../../../../platform/storage/common/storage.js';
import { TestStorageService } from '../../../../test/common/workbenchTestServices.js';
import { ForgeWorkspaceService } from '../../browser/forgeWorkspaceService.js';
import { ForgeContextType, type ForgeContextChip, type ForgeContextItem } from '../../common/forgeContextTypes.js';
import type { ForgeLayout, ForgeLayoutState, ForgePaneState, IForgeLayoutService, PanePosition } from '../../common/forgeLayoutService.js';
import type { IForgeContextService } from '../../common/forgeContextService.js';
import type { ForgeWorkspaceConfig } from '../../common/forgeWorkspaceTypes.js';
import type { IEditorService } from '../../../../services/editor/common/editorService.js';
import type { IFileService } from '../../../../../platform/files/common/files.js';

// ---------------------------------------------------------------------------
// Storage keys — must match the implementation
// ---------------------------------------------------------------------------

const WORKSPACES_STORAGE_KEY = 'forge.workspaces';

// ---------------------------------------------------------------------------
// Mock helpers — layout service
// ---------------------------------------------------------------------------

interface MockLayoutServiceState {
	layout: ForgeLayout;
	panes: ForgePaneState[];
	setLayoutCalls: ForgeLayout[];
	openChatPaneCalls: { position: PanePosition; providerId?: string }[];
}

function makeForgeLayoutService(
	initialLayout?: ForgeLayout,
	initialPanes?: ForgePaneState[],
): IForgeLayoutService & { _state: MockLayoutServiceState } {
	const state: MockLayoutServiceState = {
		layout: initialLayout ?? 'focus',
		panes: initialPanes ?? [],
		setLayoutCalls: [],
		openChatPaneCalls: [],
	};

	return {
		_serviceBrand: undefined,
		get activeLayout() { return state.layout; },
		onDidChangeLayout: Event.None,
		async setLayout(layout: ForgeLayout) {
			state.layout = layout;
			state.setLayoutCalls.push(layout);
		},
		async openChatPane(position: PanePosition, providerId?: string) {
			state.openChatPaneCalls.push({ position, providerId });
		},
		getLayoutState(): ForgeLayoutState {
			return {
				layout: state.layout,
				panes: [...state.panes],
			};
		},
		saveLayout() { },
		async restoreLayout() { },
		_state: state,
	};
}

function makeEditorService(): IEditorService {
	return {
		getEditors() { return []; },
		closeEditors() { return Promise.resolve(); },
		openEditor() { return Promise.resolve(undefined); },
	} as unknown as IEditorService;
}

function makeFileService(): IFileService {
	return {
		exists() { return Promise.resolve(true); },
	} as unknown as IFileService;
}

function makeQuadPanes(): ForgePaneState[] {
	return [
		{ position: 'tl', providerId: 'anthropic', conversationId: 'conv-1' },
		{ position: 'tr', providerId: 'openai', conversationId: 'conv-2' },
		{ position: 'bl', providerId: 'anthropic', conversationId: 'conv-3' },
		{ position: 'br', providerId: 'local', conversationId: 'conv-4' },
	];
}

function makeFocusPane(): ForgePaneState[] {
	return [
		{ position: 'tl', providerId: 'anthropic', conversationId: 'conv-focus' },
	];
}

// ---------------------------------------------------------------------------
// Mock helpers — context service
// ---------------------------------------------------------------------------

/**
 * Lightweight mock of IForgeContextService that stores chips in memory.
 * Verifies cross-service integration without the heavy real dependencies
 * (IFileService, IEditorService, IQuickInputService, etc.) that the real
 * ForgeContextService constructor requires.
 */
function makeForgeContextService(disposables: DisposableStore): IForgeContextService {
	const chipStore = new Map<string, ForgeContextChip[]>();

	function paneKey(position: PanePosition | undefined): string {
		return position ?? 'default';
	}

	const service: IForgeContextService = {
		_serviceBrand: undefined,
		onDidChangeContext: Event.None,

		getContextChips(panePosition: PanePosition | undefined): ForgeContextChip[] {
			return chipStore.get(paneKey(panePosition)) ?? [];
		},

		addContextChip(panePosition: PanePosition | undefined, item: ForgeContextItem, automatic: boolean = false): void {
			const key = paneKey(panePosition);
			const chips = chipStore.get(key) ?? [];

			const existing = chips.findIndex(c => c.item.type === item.type && c.item.label === item.label);
			if (existing !== -1) {
				chips[existing] = { item, automatic };
			} else {
				chips.push({ item, automatic });
			}
			chipStore.set(key, chips);
		},

		removeContextChip(panePosition: PanePosition | undefined, item: ForgeContextItem): void {
			const key = paneKey(panePosition);
			const chips = chipStore.get(key);
			if (!chips) {
				return;
			}
			const index = chips.findIndex(c => c.item.type === item.type && c.item.label === item.label);
			if (index !== -1) {
				chips.splice(index, 1);
				chipStore.set(key, chips);
			}
		},

		clearContext(panePosition: PanePosition | undefined): void {
			chipStore.delete(paneKey(panePosition));
		},

		async resolveContextPrompt(panePosition: PanePosition | undefined, maxTokens: number): Promise<{
			maxTokens: number;
			usedTokens: number;
			items: ForgeContextChip[];
			droppedCount: number;
		}> {
			const chips = service.getContextChips(panePosition);
			let usedTokens = 0;
			const included: ForgeContextChip[] = [];
			let droppedCount = 0;

			for (const chip of chips) {
				const cost = chip.item.tokenEstimate;
				if (usedTokens + cost <= maxTokens) {
					usedTokens += cost;
					included.push(chip);
				} else {
					droppedCount++;
				}
			}

			return { maxTokens, usedTokens, items: included, droppedCount };
		},

		async showContextPicker(): Promise<ForgeContextItem[]> {
			return [];
		},
	};

	return service;
}

// ---------------------------------------------------------------------------
// Context item factories
// ---------------------------------------------------------------------------

function makeContextItem(type: ForgeContextType, label: string, content: string, sourcePanePosition?: PanePosition): ForgeContextItem {
	return {
		type,
		label,
		content,
		tokenEstimate: Math.ceil(content.length / 4),
		sourcePanePosition,
	};
}

// ---------------------------------------------------------------------------
// Integration Tests
// ---------------------------------------------------------------------------

suite('Forge Phase 3 Integration', () => {

	let disposables: DisposableStore;
	let storageService: TestStorageService;
	let logService: NullLogService;

	setup(() => {
		disposables = new DisposableStore();
		storageService = disposables.add(new TestStorageService());
		logService = new NullLogService();
	});

	teardown(() => {
		disposables.dispose();
	});

	ensureNoDisposablesAreLeakedInTestSuite();

	// -----------------------------------------------------------------------
	// Workspace ↔ Layout integration
	// -----------------------------------------------------------------------

	test('workspace create captures layout state snapshot', async () => {
		const layoutService = makeForgeLayoutService('quad', makeQuadPanes());

		const service = disposables.add(new ForgeWorkspaceService(
			storageService,
			layoutService as IForgeLayoutService,
			makeEditorService(),
			makeFileService(),
			logService,
		));

		const workspace = await service.createWorkspace('Test');

		// The workspace should capture the layout state at creation time
		assert.strictEqual(workspace.layout, 'quad');
		assert.strictEqual(workspace.panes.length, 4);
		assert.strictEqual(workspace.panes[0].position, 'tl');
		assert.strictEqual(workspace.panes[0].providerId, 'anthropic');
		assert.strictEqual(workspace.panes[1].position, 'tr');
		assert.strictEqual(workspace.panes[1].providerId, 'openai');
		assert.strictEqual(workspace.panes[2].position, 'bl');
		assert.strictEqual(workspace.panes[2].providerId, 'anthropic');
		assert.strictEqual(workspace.panes[3].position, 'br');
		assert.strictEqual(workspace.panes[3].providerId, 'local');
	});

	test('workspace switch restores layout and opens panes', async () => {
		const quadPanes = makeQuadPanes();
		const layoutService = makeForgeLayoutService('quad', quadPanes);

		const service = disposables.add(new ForgeWorkspaceService(
			storageService,
			layoutService as IForgeLayoutService,
			makeEditorService(),
			makeFileService(),
			logService,
		));

		// Create a workspace capturing quad layout
		const workspace = await service.createWorkspace('Quad Session');

		// Simulate switching to a focus layout externally
		layoutService._state.layout = 'focus';
		layoutService._state.panes = makeFocusPane();
		layoutService._state.setLayoutCalls.length = 0;
		layoutService._state.openChatPaneCalls.length = 0;

		// Switch back to the quad workspace
		await service.switchWorkspace(workspace.id);

		// Verify setLayout was called with 'quad'
		assert.ok(
			layoutService._state.setLayoutCalls.includes('quad'),
			`Expected setLayout('quad') to be called, got: [${layoutService._state.setLayoutCalls.join(', ')}]`,
		);

		// Verify openChatPane was called for each pane in the workspace
		assert.strictEqual(
			layoutService._state.openChatPaneCalls.length,
			4,
			`Expected 4 openChatPane calls, got ${layoutService._state.openChatPaneCalls.length}`,
		);

		const calledPositions = layoutService._state.openChatPaneCalls.map(c => c.position);
		assert.ok(calledPositions.includes('tl'), 'openChatPane should be called for tl');
		assert.ok(calledPositions.includes('tr'), 'openChatPane should be called for tr');
		assert.ok(calledPositions.includes('bl'), 'openChatPane should be called for bl');
		assert.ok(calledPositions.includes('br'), 'openChatPane should be called for br');

		// Verify provider IDs were passed through
		const tlCall = layoutService._state.openChatPaneCalls.find(c => c.position === 'tl');
		assert.strictEqual(tlCall?.providerId, 'anthropic');
		const trCall = layoutService._state.openChatPaneCalls.find(c => c.position === 'tr');
		assert.strictEqual(trCall?.providerId, 'openai');
	});

	test('workspace save updates current state', async () => {
		const layoutService = makeForgeLayoutService('quad', makeQuadPanes());

		const service = disposables.add(new ForgeWorkspaceService(
			storageService,
			layoutService as IForgeLayoutService,
			makeEditorService(),
			makeFileService(),
			logService,
		));

		// Create workspace (becomes active)
		const workspace = await service.createWorkspace('Mutable Session');
		assert.strictEqual(workspace.layout, 'quad');
		assert.strictEqual(workspace.panes.length, 4);

		// Change mock layout state to simulate user changing layout
		layoutService._state.layout = 'split';
		layoutService._state.panes = [
			{ position: 'tl', providerId: 'openai', conversationId: 'new-conv-1' },
			{ position: 'tr', providerId: 'local', conversationId: 'new-conv-2' },
		];

		// Save the active workspace
		await service.saveActiveWorkspace();

		// Verify stored workspace now has the new state
		const workspaces = service.getWorkspaces();
		const saved = workspaces.find(w => w.id === workspace.id);
		assert.ok(saved, 'Workspace should still exist after save');
		assert.strictEqual(saved!.layout, 'split');
		assert.strictEqual(saved!.panes.length, 2);
		assert.strictEqual(saved!.panes[0].providerId, 'openai');
		assert.strictEqual(saved!.panes[1].providerId, 'local');
	});

	test('workspace delete clears active workspace when deleting current', async () => {
		const layoutService = makeForgeLayoutService('quad', makeQuadPanes());

		const service = disposables.add(new ForgeWorkspaceService(
			storageService,
			layoutService as IForgeLayoutService,
			makeEditorService(),
			makeFileService(),
			logService,
		));

		const workspace = await service.createWorkspace('Doomed Session');
		assert.ok(service.getActiveWorkspace(), 'Active workspace should be set after create');
		assert.strictEqual(service.getWorkspaces().length, 1);

		// Track active workspace change events
		const activeEvents: Array<ForgeWorkspaceConfig | undefined> = [];
		disposables.add(service.onDidChangeActiveWorkspace(w => { activeEvents.push(w); }));

		await service.deleteWorkspace(workspace.id);

		assert.strictEqual(service.getActiveWorkspace(), undefined, 'Active workspace should be undefined after deleting the active one');
		assert.strictEqual(service.getWorkspaces().length, 0, 'Workspace list should be empty');
		assert.ok(activeEvents.some(w => w === undefined), 'onDidChangeActiveWorkspace should have fired with undefined');
	});

	// -----------------------------------------------------------------------
	// Context service integration
	// -----------------------------------------------------------------------

	test('context chip types cover all expected sources', () => {
		const contextService = makeForgeContextService(disposables);

		// Add chips of each type
		const chips: ForgeContextItem[] = [
			makeContextItem(ForgeContextType.File, 'main.ts', 'file content'),
			makeContextItem(ForgeContextType.ActiveEditor, 'editor.ts', 'editor content'),
			makeContextItem(ForgeContextType.Selection, 'selected text', 'selection content'),
			makeContextItem(ForgeContextType.GitDiff, 'Git Diff', '- old\n+ new'),
			makeContextItem(ForgeContextType.Symbol, 'myFunction', 'function myFunction() {}'),
			makeContextItem(ForgeContextType.PaneHistory, 'Pane tl', 'conversation history', 'tl'),
		];

		for (const chip of chips) {
			contextService.addContextChip('tl', chip);
		}

		const stored = contextService.getContextChips('tl');
		assert.strictEqual(stored.length, 6, 'Should have all 6 chip types');

		// Verify each expected type is present
		const types = stored.map(c => c.item.type);
		assert.ok(types.includes(ForgeContextType.File), 'Should include file type');
		assert.ok(types.includes(ForgeContextType.ActiveEditor), 'Should include activeEditor type');
		assert.ok(types.includes(ForgeContextType.Selection), 'Should include selection type');
		assert.ok(types.includes(ForgeContextType.GitDiff), 'Should include gitDiff type');
		assert.ok(types.includes(ForgeContextType.Symbol), 'Should include symbol type');
		assert.ok(types.includes(ForgeContextType.PaneHistory), 'Should include paneHistory type');

		// Verify correct type field on each chip
		const fileChip = stored.find(c => c.item.label === 'main.ts');
		assert.strictEqual(fileChip?.item.type, ForgeContextType.File);
		const editorChip = stored.find(c => c.item.label === 'editor.ts');
		assert.strictEqual(editorChip?.item.type, ForgeContextType.ActiveEditor);
	});

	// -----------------------------------------------------------------------
	// Storage round-trip
	// -----------------------------------------------------------------------

	test('layout state persistence round-trip via storage', async () => {
		const layoutService = makeForgeLayoutService('quad', makeQuadPanes());

		// First service instance — create a workspace
		const service1 = disposables.add(new ForgeWorkspaceService(
			storageService,
			layoutService as IForgeLayoutService,
			makeEditorService(),
			makeFileService(),
			logService,
		));

		const workspace = await service1.createWorkspace('Persistent Session');

		// Verify data is in storage
		const raw = storageService.get(WORKSPACES_STORAGE_KEY, StorageScope.PROFILE);
		assert.ok(raw, 'Workspace data should be stored');

		const parsed = JSON.parse(raw!) as ForgeWorkspaceConfig[];
		assert.strictEqual(parsed.length, 1);
		assert.strictEqual(parsed[0].layout, 'quad');
		assert.strictEqual(parsed[0].panes.length, 4);

		// Dispose and construct a new service instance from the same storage
		service1.dispose();

		const service2 = disposables.add(new ForgeWorkspaceService(
			storageService,
			layoutService as IForgeLayoutService,
			makeEditorService(),
			makeFileService(),
			logService,
		));

		const workspaces = service2.getWorkspaces();
		assert.strictEqual(workspaces.length, 1, 'Workspace should survive reconstruction');
		assert.strictEqual(workspaces[0].id, workspace.id);
		assert.strictEqual(workspaces[0].name, 'Persistent Session');
		assert.strictEqual(workspaces[0].layout, 'quad');
		assert.strictEqual(workspaces[0].panes.length, 4);
		assert.strictEqual(workspaces[0].panes[0].position, 'tl');
		assert.strictEqual(workspaces[0].panes[0].providerId, 'anthropic');
	});

	// -----------------------------------------------------------------------
	// Multiple workspaces independence
	// -----------------------------------------------------------------------

	test('multiple workspaces remain independent', async () => {
		const layoutService = makeForgeLayoutService('quad', makeQuadPanes());

		const service = disposables.add(new ForgeWorkspaceService(
			storageService,
			layoutService as IForgeLayoutService,
			makeEditorService(),
			makeFileService(),
			logService,
		));

		// Create workspace A with quad layout
		const wsA = await service.createWorkspace('Workspace A');
		assert.strictEqual(wsA.layout, 'quad');
		assert.strictEqual(wsA.panes.length, 4);

		// Change mock layout to focus before creating workspace B
		layoutService._state.layout = 'focus';
		layoutService._state.panes = makeFocusPane();

		// Create workspace B with focus layout
		const wsB = await service.createWorkspace('Workspace B');
		assert.strictEqual(wsB.layout, 'focus');
		assert.strictEqual(wsB.panes.length, 1);

		// Verify both workspaces exist independently
		const workspaces = service.getWorkspaces();
		assert.strictEqual(workspaces.length, 2);

		const storedA = workspaces.find(w => w.id === wsA.id);
		const storedB = workspaces.find(w => w.id === wsB.id);

		assert.ok(storedA, 'Workspace A should exist');
		assert.ok(storedB, 'Workspace B should exist');
		assert.strictEqual(storedA!.layout, 'quad', 'Workspace A should retain quad layout');
		assert.strictEqual(storedA!.panes.length, 4, 'Workspace A should have 4 panes');
		assert.strictEqual(storedB!.layout, 'focus', 'Workspace B should retain focus layout');
		assert.strictEqual(storedB!.panes.length, 1, 'Workspace B should have 1 pane');

		// Switch to A — verify setLayout('quad') is called
		layoutService._state.setLayoutCalls.length = 0;
		layoutService._state.openChatPaneCalls.length = 0;
		await service.switchWorkspace(wsA.id);

		assert.ok(
			layoutService._state.setLayoutCalls.includes('quad'),
			`Switching to A should call setLayout('quad'), got: [${layoutService._state.setLayoutCalls.join(', ')}]`,
		);

		// Switch to B — verify setLayout('focus') is called
		layoutService._state.setLayoutCalls.length = 0;
		layoutService._state.openChatPaneCalls.length = 0;
		await service.switchWorkspace(wsB.id);

		assert.ok(
			layoutService._state.setLayoutCalls.includes('focus'),
			`Switching to B should call setLayout('focus'), got: [${layoutService._state.setLayoutCalls.join(', ')}]`,
		);
	});

	// -----------------------------------------------------------------------
	// Context + workspace cross-interaction
	// -----------------------------------------------------------------------

	test('context chips are scoped to pane positions matching workspace layout', () => {
		const contextService = makeForgeContextService(disposables);

		// Simulate quad layout — add chips to each pane position
		contextService.addContextChip('tl', makeContextItem(ForgeContextType.File, 'tl.ts', 'tl content'));
		contextService.addContextChip('tr', makeContextItem(ForgeContextType.File, 'tr.ts', 'tr content'));
		contextService.addContextChip('bl', makeContextItem(ForgeContextType.File, 'bl.ts', 'bl content'));
		contextService.addContextChip('br', makeContextItem(ForgeContextType.File, 'br.ts', 'br content'));

		// Each position should have its own independent chips
		assert.strictEqual(contextService.getContextChips('tl').length, 1);
		assert.strictEqual(contextService.getContextChips('tr').length, 1);
		assert.strictEqual(contextService.getContextChips('bl').length, 1);
		assert.strictEqual(contextService.getContextChips('br').length, 1);

		assert.strictEqual(contextService.getContextChips('tl')[0].item.label, 'tl.ts');
		assert.strictEqual(contextService.getContextChips('tr')[0].item.label, 'tr.ts');
		assert.strictEqual(contextService.getContextChips('bl')[0].item.label, 'bl.ts');
		assert.strictEqual(contextService.getContextChips('br')[0].item.label, 'br.ts');

		// Clearing one pane should not affect others
		contextService.clearContext('tl');
		assert.strictEqual(contextService.getContextChips('tl').length, 0);
		assert.strictEqual(contextService.getContextChips('tr').length, 1);
		assert.strictEqual(contextService.getContextChips('bl').length, 1);
		assert.strictEqual(contextService.getContextChips('br').length, 1);
	});

	test('context resolveContextPrompt respects token budget across chip types', async () => {
		const contextService = makeForgeContextService(disposables);

		// Add chips with known token estimates
		// 100 chars / 4 = 25 tokens each
		contextService.addContextChip('tl', makeContextItem(ForgeContextType.File, 'big.ts', 'x'.repeat(100)));
		contextService.addContextChip('tl', makeContextItem(ForgeContextType.GitDiff, 'diff', 'y'.repeat(100)));
		contextService.addContextChip('tl', makeContextItem(ForgeContextType.Selection, 'sel', 'z'.repeat(100)));

		// Budget for 2 items (50 tokens)
		const budget = await contextService.resolveContextPrompt('tl', 50);

		assert.strictEqual(budget.items.length, 2, 'Should fit exactly 2 items');
		assert.strictEqual(budget.droppedCount, 1, 'Should drop 1 item');
		assert.ok(budget.usedTokens <= 50, `usedTokens (${budget.usedTokens}) should be <= 50`);
		assert.ok(budget.usedTokens > 0, 'usedTokens should be > 0');
	});

	// -----------------------------------------------------------------------
	// Workspace save + switch round-trip preserves pane state
	// -----------------------------------------------------------------------

	test('workspace save then switch round-trip preserves updated pane state', async () => {
		const layoutService = makeForgeLayoutService('quad', makeQuadPanes());

		const service = disposables.add(new ForgeWorkspaceService(
			storageService,
			layoutService as IForgeLayoutService,
			makeEditorService(),
			makeFileService(),
			logService,
		));

		// Create workspace with quad layout
		const workspace = await service.createWorkspace('Round Trip');

		// Mutate the layout state (simulate user changing providers)
		layoutService._state.panes = [
			{ position: 'tl', providerId: 'openai', conversationId: 'new-conv-1' },
			{ position: 'tr', providerId: 'openai', conversationId: 'new-conv-2' },
			{ position: 'bl', providerId: 'local', conversationId: 'new-conv-3' },
			{ position: 'br', providerId: 'local', conversationId: 'new-conv-4' },
		];

		// Save the updated state
		await service.saveActiveWorkspace();

		// Create a second workspace and switch to it
		layoutService._state.layout = 'focus';
		layoutService._state.panes = makeFocusPane();
		await service.createWorkspace('Other');

		// Switch back to the first workspace
		layoutService._state.openChatPaneCalls.length = 0;
		await service.switchWorkspace(workspace.id);

		// Verify that the openChatPane calls carry the updated provider IDs
		const tlCall = layoutService._state.openChatPaneCalls.find(c => c.position === 'tl');
		assert.strictEqual(tlCall?.providerId, 'openai', 'tl should use the saved provider (openai)');

		const brCall = layoutService._state.openChatPaneCalls.find(c => c.position === 'br');
		assert.strictEqual(brCall?.providerId, 'local', 'br should use the saved provider (local)');
	});
});
