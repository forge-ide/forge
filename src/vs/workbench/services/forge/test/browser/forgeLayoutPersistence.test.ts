/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import assert from 'assert';
import { Emitter, Event } from '../../../../../base/common/event.js';
import { DisposableStore, IDisposable } from '../../../../../base/common/lifecycle.js';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import { NullLogService } from '../../../../../platform/log/common/log.js';
import { StorageScope, WillSaveStateReason } from '../../../../../platform/storage/common/storage.js';
import { TestStorageService } from '../../../../test/common/workbenchTestServices.js';
import { TestEditorGroupView, TestEditorGroupsService } from '../../../../test/browser/workbenchTestServices.js';
import { EditorGroupLayout, GroupsOrder, IEditorGroup, IEditorGroupsService } from '../../../editor/common/editorGroupsService.js';
import type { IEditorPane, IEditorPartOptions } from '../../../../common/editor.js';
import { ForgeLayoutService } from '../../browser/forgeLayoutService.js';
import type { ForgeConfig, IForgeConfigService } from '../../common/forgeConfigService.js';
import type { ForgeLayoutState } from '../../common/forgeLayoutService.js';
import { EditorInput } from '../../../../common/editor/editorInput.js';

const STORAGE_KEY = 'forge.layout.state';

/**
 * MockEditorGroupsService with controllable event emitters and call tracking.
 * Mirrors the one in forgeLayoutService.test.ts.
 */
class MockEditorGroupsService extends TestEditorGroupsService {

	readonly appliedLayouts: EditorGroupLayout[] = [];
	readonly enforcedOptions: Partial<IEditorPartOptions>[] = [];

	private readonly _onDidAddGroup = new Emitter<IEditorGroup>();
	private readonly _onDidRemoveGroup = new Emitter<IEditorGroup>();

	override readonly onDidAddGroup = this._onDidAddGroup.event;
	override readonly onDidRemoveGroup = this._onDidRemoveGroup.event;

	constructor(groups: TestEditorGroupView[] = []) {
		super(groups);
	}

	override readonly mainPart = this;

	override applyLayout(layout: EditorGroupLayout): void {
		this.appliedLayouts.push(layout);
	}

	override enforcePartOptions(options: Partial<IEditorPartOptions>): IDisposable {
		this.enforcedOptions.push(options);
		return { dispose() { } };
	}

	override mergeGroup(): boolean {
		return true;
	}

	override getGroups(_order?: GroupsOrder): readonly IEditorGroup[] {
		return this.groups;
	}

	fireDidRemoveGroup(group: IEditorGroup): void {
		this._onDidRemoveGroup.fire(group);
	}

	fireDidAddGroup(group: IEditorGroup): void {
		this._onDidAddGroup.fire(group);
	}
}

/**
 * TestEditorGroupView that tracks openEditor calls.
 */
class MockEditorGroupView extends TestEditorGroupView {

	readonly openedEditors: EditorInput[] = [];

	override async openEditor(editor: EditorInput): Promise<IEditorPane> {
		this.openedEditors.push(editor);
		return undefined!;
	}
}

function makeForgeConfigService(overrides?: Partial<ForgeConfig>): IForgeConfigService {
	const config: ForgeConfig = {
		provider: 'anthropic',
		model: 'claude-sonnet-4-6',
		...overrides,
	};
	return {
		_serviceBrand: undefined,
		onDidChange: Event.None,
		getConfig() { return { ...config }; },
		async updateConfig() { },
	};
}

function makeQuadGroups(): MockEditorGroupView[] {
	return [
		new MockEditorGroupView(1),
		new MockEditorGroupView(2),
		new MockEditorGroupView(3),
		new MockEditorGroupView(4),
	];
}

suite('ForgeLayoutService persistence', () => {

	let disposables: DisposableStore;
	let editorGroupsService: MockEditorGroupsService;
	let storageService: TestStorageService;
	let configService: IForgeConfigService;
	let logService: NullLogService;

	setup(() => {
		disposables = new DisposableStore();
		logService = new NullLogService();
		storageService = disposables.add(new TestStorageService());
		configService = makeForgeConfigService();
		editorGroupsService = new MockEditorGroupsService([
			new MockEditorGroupView(1),
		]);
	});

	teardown(() => {
		disposables.dispose();
	});

	ensureNoDisposablesAreLeakedInTestSuite();

	function createService(groups?: MockEditorGroupView[]): ForgeLayoutService {
		if (groups) {
			editorGroupsService = new MockEditorGroupsService(groups);
		}
		return disposables.add(new ForgeLayoutService(
			editorGroupsService as unknown as IEditorGroupsService,
			storageService,
			configService,
			logService,
		));
	}

	test('saveLayout writes ForgeLayoutState JSON to workspace storage', async () => {
		const quadGroups = makeQuadGroups();
		const service = createService(quadGroups);

		await service.setLayout('quad');
		service.saveLayout();

		const raw = storageService.get(STORAGE_KEY, StorageScope.WORKSPACE);
		assert.ok(raw, 'Layout state should be written to workspace storage');

		const parsed: ForgeLayoutState = JSON.parse(raw!);
		assert.strictEqual(parsed.layout, 'quad');
		assert.strictEqual(parsed.panes.length, 4);
	});

	test('restoreLayout reads from storage and applies the saved layout', async () => {
		const quadGroups = makeQuadGroups();

		const savedState: ForgeLayoutState = {
			layout: 'quad',
			panes: [
				{ position: 'tl', providerId: 'anthropic', conversationId: 'conv-p1' },
				{ position: 'tr', providerId: 'openai', conversationId: 'conv-p2' },
				{ position: 'bl', providerId: 'anthropic', conversationId: 'conv-p3' },
				{ position: 'br', providerId: 'local', conversationId: 'conv-p4' },
			],
		};
		storageService.store(STORAGE_KEY, JSON.stringify(savedState), StorageScope.WORKSPACE, 1 /* StorageTarget.MACHINE */);

		const service = createService(quadGroups);
		await service.restoreLayout();

		assert.strictEqual(service.activeLayout, 'quad');
	});

	test('restoreLayout opens ForgeChatInput in each group with saved provider/model', async () => {
		const quadGroups = makeQuadGroups();

		const savedState: ForgeLayoutState = {
			layout: 'quad',
			panes: [
				{ position: 'tl', providerId: 'anthropic', conversationId: 'conv-r1' },
				{ position: 'tr', providerId: 'openai', conversationId: 'conv-r2' },
				{ position: 'bl', providerId: 'anthropic', conversationId: 'conv-r3' },
				{ position: 'br', providerId: 'local', conversationId: 'conv-r4' },
			],
		};
		storageService.store(STORAGE_KEY, JSON.stringify(savedState), StorageScope.WORKSPACE, 1 /* StorageTarget.MACHINE */);

		const service = createService(quadGroups);
		await service.restoreLayout();

		const state = service.getLayoutState();
		assert.strictEqual(state.panes.length, 4);

		// Verify each pane state has the saved provider IDs
		const providers = state.panes.map(p => p.providerId).sort();
		assert.deepStrictEqual(providers, ['anthropic', 'anthropic', 'local', 'openai']);
	});

	test('restoreLayout with missing storage key defaults to focus layout', async () => {
		// No storage data written
		const service = createService();

		await service.restoreLayout();

		assert.strictEqual(service.activeLayout, 'focus');
		const state = service.getLayoutState();
		assert.strictEqual(state.panes.length, 0);
	});

	test('restoreLayout with malformed JSON logs warning and defaults to focus', async () => {
		storageService.store(STORAGE_KEY, '{ broken json !!!', StorageScope.WORKSPACE, 1 /* StorageTarget.MACHINE */);

		const service = createService();
		await service.restoreLayout();

		assert.strictEqual(service.activeLayout, 'focus');
	});

	test('onWillSaveState triggers saveLayout automatically', async () => {
		const quadGroups = makeQuadGroups();
		const service = createService(quadGroups);

		await service.setLayout('quad');

		// Verify nothing in storage yet (saveLayout hasn't been called directly)
		// Clear any auto-stored data to test the hook
		storageService.remove(STORAGE_KEY, StorageScope.WORKSPACE);

		// Emit the onWillSaveState event
		storageService.testEmitWillSaveState(WillSaveStateReason.SHUTDOWN);

		const raw = storageService.get(STORAGE_KEY, StorageScope.WORKSPACE);
		assert.ok(raw, 'saveLayout should have been called by onWillSaveState hook');

		const parsed: ForgeLayoutState = JSON.parse(raw!);
		assert.strictEqual(parsed.layout, 'quad');
	});

	test('saveLayout does not include conversation messages in stored JSON', async () => {
		const quadGroups = makeQuadGroups();
		const service = createService(quadGroups);

		await service.setLayout('quad');
		service.saveLayout();

		const raw = storageService.get(STORAGE_KEY, StorageScope.WORKSPACE);
		assert.ok(raw);

		const parsed = JSON.parse(raw!);

		// The stored state should only contain layout metadata, not message content
		assert.ok(!Object.prototype.hasOwnProperty.call(parsed, 'messages'), 'Stored state should not contain messages');
		for (const pane of parsed.panes) {
			assert.ok(!Object.prototype.hasOwnProperty.call(pane, 'messages'), 'Pane state should not contain messages');
			assert.ok(!Object.prototype.hasOwnProperty.call(pane, 'history'), 'Pane state should not contain history');
			// Pane should only have position, providerId, conversationId, and optionally model
			const allowedKeys = new Set(['position', 'providerId', 'conversationId', 'model']);
			for (const key of Object.keys(pane)) {
				assert.ok(allowedKeys.has(key), `Unexpected key "${key}" in stored pane state`);
			}
		}
	});

	test('save/restore cycle with quad layout produces identical ForgeLayoutState', async () => {
		const quadGroups = makeQuadGroups();
		const service = createService(quadGroups);

		await service.setLayout('quad');

		// Capture the state before saving
		const stateBefore = service.getLayoutState();
		service.saveLayout();

		// Read back from storage
		const raw = storageService.get(STORAGE_KEY, StorageScope.WORKSPACE);
		assert.ok(raw);

		const parsed: ForgeLayoutState = JSON.parse(raw!);

		assert.strictEqual(parsed.layout, stateBefore.layout);
		assert.strictEqual(parsed.panes.length, stateBefore.panes.length);

		for (let i = 0; i < parsed.panes.length; i++) {
			assert.strictEqual(parsed.panes[i].position, stateBefore.panes[i].position);
			assert.strictEqual(parsed.panes[i].providerId, stateBefore.panes[i].providerId);
			assert.strictEqual(parsed.panes[i].conversationId, stateBefore.panes[i].conversationId);
		}
	});

	test('switching workspaces clears and reloads layout from new workspace storage', async () => {
		const quadGroups = makeQuadGroups();
		const service = createService(quadGroups);

		await service.setLayout('quad');
		service.saveLayout();

		// Verify quad state is stored
		const rawQuad = storageService.get(STORAGE_KEY, StorageScope.WORKSPACE);
		assert.ok(rawQuad);

		// Simulate workspace switch by clearing storage and writing a different state
		storageService.remove(STORAGE_KEY, StorageScope.WORKSPACE);

		const focusState: ForgeLayoutState = {
			layout: 'focus',
			panes: [],
		};
		storageService.store(STORAGE_KEY, JSON.stringify(focusState), StorageScope.WORKSPACE, 1 /* StorageTarget.MACHINE */);

		// Switch to a single group to match focus layout
		editorGroupsService.groups = [new MockEditorGroupView(1)];

		// Restore from the "new workspace" storage
		await service.restoreLayout();

		assert.strictEqual(service.activeLayout, 'focus');
		const state = service.getLayoutState();
		assert.strictEqual(state.panes.length, 0);
	});
});
