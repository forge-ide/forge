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
import type { ForgeLayout, ForgeLayoutState, ForgePaneState, IForgeLayoutService } from '../../common/forgeLayoutService.js';
import type { ForgeWorkspaceConfig } from '../../common/forgeWorkspaceTypes.js';
import { ForgeWorkspaceService } from '../../browser/forgeWorkspaceService.js';
import type { IEditorService } from '../../../../services/editor/common/editorService.js';
import type { IFileService } from '../../../../../platform/files/common/files.js';

// ---------------------------------------------------------------------------
// Storage keys — must match the implementation
// ---------------------------------------------------------------------------

const WORKSPACES_STORAGE_KEY = 'forge.workspaces';
const ACTIVE_WORKSPACE_STORAGE_KEY = 'forge.activeWorkspaceId';

// ---------------------------------------------------------------------------
// Mock helpers
// ---------------------------------------------------------------------------

/**
 * Mock layout service that returns a configurable layout state and tracks
 * setLayout / openChatPane calls for verification.
 */
interface MockLayoutServiceState {
	layout: ForgeLayout;
	panes: ForgePaneState[];
	setLayoutCalls: ForgeLayout[];
	openChatPaneCalls: { position: string; providerId?: string }[];
}

function makeForgeLayoutService(initialLayout?: ForgeLayout, initialPanes?: ForgePaneState[]): IForgeLayoutService & { _state: MockLayoutServiceState } {
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
		async openChatPane(position, providerId) {
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

suite('ForgeWorkspaceService', () => {

	let disposables: DisposableStore;
	let storageService: TestStorageService;
	let layoutService: ReturnType<typeof makeForgeLayoutService>;
	let logService: NullLogService;

	setup(() => {
		disposables = new DisposableStore();
		storageService = disposables.add(new TestStorageService());
		layoutService = makeForgeLayoutService();
		logService = new NullLogService();
	});

	teardown(() => {
		disposables.dispose();
	});

	ensureNoDisposablesAreLeakedInTestSuite();

	function createService(): ForgeWorkspaceService {
		const service = new ForgeWorkspaceService(
			storageService,
			layoutService as IForgeLayoutService,
			makeEditorService(),
			makeFileService(),
			logService,
		);
		return disposables.add(service);
	}

	// -----------------------------------------------------------------------
	// createWorkspace
	// -----------------------------------------------------------------------

	suite('createWorkspace', () => {

		test('stores a new workspace with given name', async () => {
			const service = createService();

			const workspace = await service.createWorkspace('My Session');

			assert.strictEqual(workspace.name, 'My Session');
		});

		test('captures current layout state from ForgeLayoutService', async () => {
			layoutService._state.layout = 'quad';
			layoutService._state.panes = makeQuadPanes();
			const service = createService();

			const workspace = await service.createWorkspace('Quad Session');

			assert.strictEqual(workspace.layout, 'quad');
			assert.strictEqual(workspace.panes.length, 4);
			assert.strictEqual(workspace.panes[0].position, 'tl');
			assert.strictEqual(workspace.panes[0].providerId, 'anthropic');
			assert.strictEqual(workspace.panes[1].position, 'tr');
			assert.strictEqual(workspace.panes[1].providerId, 'openai');
			assert.strictEqual(workspace.panes[2].position, 'bl');
			assert.strictEqual(workspace.panes[3].position, 'br');
		});

		test('generates a unique ID for each workspace', async () => {
			const service = createService();

			const ws1 = await service.createWorkspace('Session 1');
			const ws2 = await service.createWorkspace('Session 2');
			const ws3 = await service.createWorkspace('Session 3');

			assert.notStrictEqual(ws1.id, ws2.id);
			assert.notStrictEqual(ws2.id, ws3.id);
			assert.notStrictEqual(ws1.id, ws3.id);
			assert.ok(ws1.id.length > 0, 'ID should not be empty');
		});

		test('fires onDidChangeWorkspaces event', async () => {
			const service = createService();

			let firedCount = 0;
			disposables.add(service.onDidChangeWorkspaces(() => { firedCount++; }));

			await service.createWorkspace('Event Test');

			assert.strictEqual(firedCount, 1);
		});

		test('created workspace appears in getWorkspaces()', async () => {
			const service = createService();

			const workspace = await service.createWorkspace('Listed Session');
			const workspaces = service.getWorkspaces();

			assert.strictEqual(workspaces.length, 1);
			assert.strictEqual(workspaces[0].id, workspace.id);
			assert.strictEqual(workspaces[0].name, 'Listed Session');
		});

		test('sets createdAt to current timestamp', async () => {
			const before = Date.now();
			const service = createService();

			const workspace = await service.createWorkspace('Timestamped');

			const after = Date.now();
			assert.ok(workspace.createdAt >= before, `createdAt (${workspace.createdAt}) should be >= ${before}`);
			assert.ok(workspace.createdAt <= after, `createdAt (${workspace.createdAt}) should be <= ${after}`);
		});
	});

	// -----------------------------------------------------------------------
	// switchWorkspace
	// -----------------------------------------------------------------------

	suite('switchWorkspace', () => {

		test('applies saved layout via ForgeLayoutService.setLayout()', async () => {
			layoutService._state.layout = 'quad';
			layoutService._state.panes = makeQuadPanes();
			const service = createService();

			const workspace = await service.createWorkspace('Switch Target');

			// Create a second workspace so activeWorkspaceId differs from workspace.id
			// (switchWorkspace early-returns when switching to the already-active workspace)
			await service.createWorkspace('Other');

			// Reset layout mock state
			layoutService._state.setLayoutCalls.length = 0;
			layoutService._state.layout = 'focus';
			layoutService._state.panes = [];

			await service.switchWorkspace(workspace.id);

			assert.ok(
				layoutService._state.setLayoutCalls.includes('quad'),
				`setLayout should have been called with "quad", got: [${layoutService._state.setLayoutCalls.join(', ')}]`
			);
		});

		test('updates activeWorkspace to switched workspace', async () => {
			const service = createService();

			const workspace = await service.createWorkspace('Active Check');

			// Create another workspace so activeWorkspace changes
			const workspace2 = await service.createWorkspace('Other');
			assert.strictEqual(service.getActiveWorkspace()!.id, workspace2.id);

			await service.switchWorkspace(workspace.id);

			const active = service.getActiveWorkspace();
			assert.ok(active, 'Active workspace should be defined');
			assert.strictEqual(active!.id, workspace.id);
			assert.strictEqual(active!.name, 'Active Check');
		});

		test('fires onDidChangeActiveWorkspace event', async () => {
			const service = createService();
			const workspace = await service.createWorkspace('Event Switch');

			// Create second workspace to clear events from createWorkspace
			await service.createWorkspace('Other');

			const captured: Array<ForgeWorkspaceConfig | undefined> = [];
			disposables.add(service.onDidChangeActiveWorkspace(w => { captured.push(w); }));

			await service.switchWorkspace(workspace.id);

			// switchWorkspace fires twice: once from saveActiveWorkspace (outgoing)
			// and once for the incoming workspace
			assert.ok(captured.length >= 1, 'At least one event should fire');
			const lastEvent = captured[captured.length - 1];
			assert.ok(lastEvent, 'Last event payload should be defined');
			assert.strictEqual(lastEvent!.id, workspace.id);
		});

		test('throws or warns for nonexistent workspace ID', async () => {
			const service = createService();

			// The implementation logs a warning and returns early for nonexistent IDs.
			// Active workspace should remain undefined.
			await service.switchWorkspace('nonexistent-id');

			const active = service.getActiveWorkspace();
			assert.strictEqual(active, undefined, 'Active workspace should not be set to a nonexistent workspace');
		});
	});

	// -----------------------------------------------------------------------
	// saveActiveWorkspace
	// -----------------------------------------------------------------------

	suite('saveActiveWorkspace', () => {

		test('updates stored state with current layout', async () => {
			layoutService._state.layout = 'quad';
			layoutService._state.panes = makeQuadPanes();
			const service = createService();

			const workspace = await service.createWorkspace('Save Target');

			// Change layout state after workspace was created
			layoutService._state.layout = 'split';
			layoutService._state.panes = [
				{ position: 'tl', providerId: 'openai', conversationId: 'new-conv-1' },
				{ position: 'tr', providerId: 'local', conversationId: 'new-conv-2' },
			];

			await service.saveActiveWorkspace();

			// Retrieve from getWorkspaces and verify updated state
			const workspaces = service.getWorkspaces();
			const saved = workspaces.find(w => w.id === workspace.id);
			assert.ok(saved, 'Workspace should still exist');
			assert.strictEqual(saved!.layout, 'split');
			assert.strictEqual(saved!.panes.length, 2);
			assert.strictEqual(saved!.panes[0].providerId, 'openai');
		});

		test('is a no-op when no active workspace exists', async () => {
			// Construct service with no prior workspaces — activeWorkspaceId is undefined
			const service = createService();

			let firedCount = 0;
			disposables.add(service.onDidChangeWorkspaces(() => { firedCount++; }));

			await service.saveActiveWorkspace();

			assert.strictEqual(firedCount, 0, 'No event should fire for a no-op save');
		});

		test('fires onDidChangeWorkspaces after save', async () => {
			const service = createService();
			await service.createWorkspace('Save Event');

			let firedCount = 0;
			disposables.add(service.onDidChangeWorkspaces(() => { firedCount++; }));

			await service.saveActiveWorkspace();

			assert.ok(firedCount >= 1, 'onDidChangeWorkspaces should fire after save');
		});
	});

	// -----------------------------------------------------------------------
	// deleteWorkspace
	// -----------------------------------------------------------------------

	suite('deleteWorkspace', () => {

		test('removes workspace from storage', async () => {
			const service = createService();

			const workspace = await service.createWorkspace('To Delete');
			assert.strictEqual(service.getWorkspaces().length, 1);

			await service.deleteWorkspace(workspace.id);

			assert.strictEqual(service.getWorkspaces().length, 0);
		});

		test('fires onDidChangeWorkspaces event', async () => {
			const service = createService();
			const workspace = await service.createWorkspace('Delete Event');

			let firedCount = 0;
			disposables.add(service.onDidChangeWorkspaces(() => { firedCount++; }));

			await service.deleteWorkspace(workspace.id);

			assert.strictEqual(firedCount, 1);
		});

		test('clears activeWorkspace when deleting the active one', async () => {
			const service = createService();

			const workspace = await service.createWorkspace('Active Delete');
			// createWorkspace sets it as active
			assert.ok(service.getActiveWorkspace(), 'Should have active workspace before delete');

			const captured: Array<ForgeWorkspaceConfig | undefined> = [];
			disposables.add(service.onDidChangeActiveWorkspace(w => { captured.push(w); }));

			await service.deleteWorkspace(workspace.id);

			assert.strictEqual(service.getActiveWorkspace(), undefined, 'Active workspace should be cleared');
			assert.ok(captured.some(w => w === undefined), 'Should fire onDidChangeActiveWorkspace with undefined');
		});

		test('deleted workspace no longer appears in getWorkspaces()', async () => {
			const service = createService();

			const ws1 = await service.createWorkspace('Keep');
			const ws2 = await service.createWorkspace('Delete');
			const ws3 = await service.createWorkspace('Also Keep');

			await service.deleteWorkspace(ws2.id);

			const remaining = service.getWorkspaces();
			assert.strictEqual(remaining.length, 2);
			assert.ok(remaining.some(w => w.id === ws1.id), 'ws1 should remain');
			assert.ok(remaining.some(w => w.id === ws3.id), 'ws3 should remain');
			assert.ok(!remaining.some(w => w.id === ws2.id), 'ws2 should be removed');
		});
	});

	// -----------------------------------------------------------------------
	// renameWorkspace
	// -----------------------------------------------------------------------

	suite('renameWorkspace', () => {

		test('updates the name in storage', async () => {
			const service = createService();
			const workspace = await service.createWorkspace('Old Name');

			await service.renameWorkspace(workspace.id, 'New Name');

			const workspaces = service.getWorkspaces();
			const renamed = workspaces.find(w => w.id === workspace.id);
			assert.ok(renamed, 'Workspace should still exist');
			assert.strictEqual(renamed!.name, 'New Name');
		});

		test('fires onDidChangeWorkspaces event', async () => {
			const service = createService();
			const workspace = await service.createWorkspace('Before Rename');

			let firedCount = 0;
			disposables.add(service.onDidChangeWorkspaces(() => { firedCount++; }));

			await service.renameWorkspace(workspace.id, 'After Rename');

			assert.strictEqual(firedCount, 1);
		});

		test('renamed workspace has new name in getWorkspaces()', async () => {
			const service = createService();

			const ws1 = await service.createWorkspace('Alpha');
			const ws2 = await service.createWorkspace('Beta');

			await service.renameWorkspace(ws1.id, 'Alpha Renamed');

			const workspaces = service.getWorkspaces();
			const alpha = workspaces.find(w => w.id === ws1.id);
			const beta = workspaces.find(w => w.id === ws2.id);

			assert.ok(alpha, 'Alpha should exist');
			assert.ok(beta, 'Beta should exist');
			assert.strictEqual(alpha!.name, 'Alpha Renamed');
			assert.strictEqual(beta!.name, 'Beta', 'Other workspace should be unchanged');
		});
	});

	// -----------------------------------------------------------------------
	// conversation persistence
	// -----------------------------------------------------------------------

	suite('conversation persistence', () => {

		test('created workspace includes empty conversations array when no conversations', async () => {
			const service = createService();

			const workspace = await service.createWorkspace('No Conversations');

			assert.ok(Array.isArray(workspace.conversations), 'conversations should be an array');
			assert.strictEqual(workspace.conversations.length, 0);
		});

		test('conversations are preserved through create and getWorkspaces', async () => {
			layoutService._state.layout = 'quad';
			layoutService._state.panes = makeQuadPanes();
			const service = createService();

			const workspace = await service.createWorkspace('With Conversations');

			const workspaces = service.getWorkspaces();
			const found = workspaces.find(w => w.id === workspace.id);
			assert.ok(found, 'Workspace should be in list');
			assert.deepStrictEqual(found!.conversations, workspace.conversations);
		});
	});

	// -----------------------------------------------------------------------
	// persistence across restarts
	// -----------------------------------------------------------------------

	suite('persistence across restarts', () => {

		test('workspaces survive service reconstruction from storage', async () => {
			layoutService._state.layout = 'quad';
			layoutService._state.panes = makeQuadPanes();

			// First service instance — create workspaces
			const service1 = createService();

			const ws1 = await service1.createWorkspace('Persistent 1');
			const ws2 = await service1.createWorkspace('Persistent 2');

			// Dispose first instance (but storage persists)
			service1.dispose();

			// Construct a second service from the same storage
			const service2 = new ForgeWorkspaceService(
				storageService,
				layoutService as IForgeLayoutService,
				makeEditorService(),
				makeFileService(),
				logService,
			);
			disposables.add(service2);

			const workspaces = service2.getWorkspaces();
			assert.strictEqual(workspaces.length, 2, 'Both workspaces should survive reconstruction');
			assert.ok(workspaces.some(w => w.id === ws1.id), 'ws1 should be present');
			assert.ok(workspaces.some(w => w.id === ws2.id), 'ws2 should be present');
			assert.ok(workspaces.some(w => w.name === 'Persistent 1'));
			assert.ok(workspaces.some(w => w.name === 'Persistent 2'));
		});

		test('active workspace ID persists in workspace-scoped storage', async () => {
			const service = createService();
			const workspace = await service.createWorkspace('Active Persist');

			// createWorkspace sets the active workspace automatically
			const storedId = storageService.get(ACTIVE_WORKSPACE_STORAGE_KEY, StorageScope.WORKSPACE);
			assert.strictEqual(storedId, workspace.id, 'Active workspace ID should be in workspace-scoped storage');
		});

		test('workspace list persists in profile-scoped storage', async () => {
			const service = createService();
			await service.createWorkspace('Profile Scoped');

			const raw = storageService.get(WORKSPACES_STORAGE_KEY, StorageScope.PROFILE);
			assert.ok(raw, 'Workspace list should be in profile-scoped storage');

			const parsed = JSON.parse(raw!) as ForgeWorkspaceConfig[];
			assert.ok(Array.isArray(parsed), 'Stored value should be an array');
			assert.strictEqual(parsed.length, 1);
			assert.strictEqual(parsed[0].name, 'Profile Scoped');
		});
	});
});
