/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import assert from 'assert';
import { Emitter, Event } from '../../../../../base/common/event.js';
import { DisposableStore, IDisposable } from '../../../../../base/common/lifecycle.js';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import { NullLogService } from '../../../../../platform/log/common/log.js';
import { TestStorageService } from '../../../../test/common/workbenchTestServices.js';
import { TestEditorGroupView, TestEditorGroupsService } from '../../../../test/browser/workbenchTestServices.js';
import { EditorGroupLayout, GroupOrientation, GroupsOrder, IEditorGroup, IEditorGroupsService } from '../../../editor/common/editorGroupsService.js';
import type { IEditorPane, IEditorPartOptions } from '../../../../common/editor.js';
import { ForgeLayoutService } from '../../browser/forgeLayoutService.js';
import { ForgeLayout, IForgeLayoutService } from '../../common/forgeLayoutService.js';
import type { ForgeConfig, IForgeConfigService } from '../../common/forgeConfigService.js';
import { EditorInput } from '../../../../common/editor/editorInput.js';
import { ForgeChatInput } from '../../../../browser/parts/editor/forgeChat/forgeChatInput.js';

/**
 * Extends TestEditorGroupsService with controllable event emitters and
 * tracking of applyLayout / enforcePartOptions calls, which the base
 * test class stubs as no-ops.
 */
class MockEditorGroupsService extends TestEditorGroupsService {

	readonly appliedLayouts: EditorGroupLayout[] = [];
	readonly enforcedOptions: Partial<IEditorPartOptions>[] = [];
	readonly mergedGroups: { source: number; target: number }[] = [];

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
		const disposable = { disposed: false, dispose() { this.disposed = true; } };
		return disposable;
	}

	override mergeGroup(source: number | IEditorGroup, target: number | IEditorGroup): boolean {
		const sourceId = typeof source === 'number' ? source : source.id;
		const targetId = typeof target === 'number' ? target : target.id;
		this.mergedGroups.push({ source: sourceId, target: targetId });
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
 * TestEditorGroupView extended to track openEditor calls.
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

suite('ForgeLayoutService', () => {

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

		// Default: single group (focus layout)
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
		const service = new ForgeLayoutService(
			editorGroupsService as unknown as IEditorGroupsService,
			storageService,
			configService,
			logService,
		);
		return disposables.add(service);
	}

	function makeQuadGroups(): MockEditorGroupView[] {
		return [
			new MockEditorGroupView(1),
			new MockEditorGroupView(2),
			new MockEditorGroupView(3),
			new MockEditorGroupView(4),
		];
	}

	test('IForgeLayoutService decorator token is a valid service identifier', () => {
		assert.ok(IForgeLayoutService);
		assert.strictEqual(typeof IForgeLayoutService, 'object');
		assert.strictEqual(IForgeLayoutService.toString(), 'forgeLayoutService');
	});

	test('initial activeLayout is "focus"', () => {
		const service = createService();
		assert.strictEqual(service.activeLayout, 'focus');
	});

	suite('setLayout', () => {

		test('setLayout("quad") applies 2x2 grid layout to editor groups service', async () => {
			const quadGroups = makeQuadGroups();
			const service = createService(quadGroups);

			await service.setLayout('quad');

			assert.strictEqual(editorGroupsService.appliedLayouts.length, 1);
			const layout = editorGroupsService.appliedLayouts[0];
			assert.strictEqual(layout.orientation, GroupOrientation.HORIZONTAL);
			assert.strictEqual(layout.groups.length, 2);
			// Each row has 2 groups
			assert.ok(layout.groups[0].groups !== undefined);
			assert.ok(layout.groups[1].groups !== undefined);
			assert.strictEqual(layout.groups[0].groups!.length, 2);
			assert.strictEqual(layout.groups[1].groups!.length, 2);
		});

		test('setLayout("quad") calls mainPart.enforcePartOptions with closeEmptyGroups false', async () => {
			const quadGroups = makeQuadGroups();
			const service = createService(quadGroups);

			await service.setLayout('quad');

			assert.strictEqual(editorGroupsService.enforcedOptions.length, 1);
			assert.strictEqual(editorGroupsService.enforcedOptions[0].closeEmptyGroups, false);
		});

		test('setLayout("split") applies 1x2 horizontal layout', async () => {
			const groups = [new MockEditorGroupView(1), new MockEditorGroupView(2)];
			const service = createService(groups);

			await service.setLayout('split');

			assert.strictEqual(editorGroupsService.appliedLayouts.length, 1);
			const layout = editorGroupsService.appliedLayouts[0];
			assert.strictEqual(layout.orientation, GroupOrientation.HORIZONTAL);
			assert.strictEqual(layout.groups.length, 2);
		});

		test('setLayout("split") calls mainPart.enforcePartOptions with closeEmptyGroups false', async () => {
			const groups = [new MockEditorGroupView(1), new MockEditorGroupView(2)];
			const service = createService(groups);

			await service.setLayout('split');

			assert.strictEqual(editorGroupsService.enforcedOptions.length, 1);
			assert.strictEqual(editorGroupsService.enforcedOptions[0].closeEmptyGroups, false);
		});

		test('setLayout("focus") applies single group layout', async () => {
			const groups = [new MockEditorGroupView(1), new MockEditorGroupView(2)];
			const service = createService(groups);

			// Switch to split first so we can go to focus
			await service.setLayout('split');
			editorGroupsService.appliedLayouts.length = 0;

			await service.setLayout('focus');

			assert.strictEqual(editorGroupsService.appliedLayouts.length, 1);
			const layout = editorGroupsService.appliedLayouts[0];
			assert.strictEqual(layout.orientation, GroupOrientation.HORIZONTAL);
			assert.strictEqual(layout.groups.length, 1);
		});

		test('setLayout("code+ai") applies 1x2 layout with chat in right pane', async () => {
			const groups = [new MockEditorGroupView(1), new MockEditorGroupView(2)];
			const service = createService(groups);

			await service.setLayout('code+ai');

			assert.strictEqual(editorGroupsService.appliedLayouts.length, 1);
			const layout = editorGroupsService.appliedLayouts[0];
			assert.strictEqual(layout.orientation, GroupOrientation.HORIZONTAL);
			assert.strictEqual(layout.groups.length, 2);

			// The right group should have a chat input opened
			const rightGroup = groups[1];
			assert.strictEqual(rightGroup.openedEditors.length, 1);
			assert.ok(rightGroup.openedEditors[0] instanceof ForgeChatInput);
		});

		test('setLayout("code+ai") calls mainPart.enforcePartOptions with closeEmptyGroups false', async () => {
			const groups = [new MockEditorGroupView(1), new MockEditorGroupView(2)];
			const service = createService(groups);

			await service.setLayout('code+ai');

			assert.strictEqual(editorGroupsService.enforcedOptions.length, 1);
			assert.strictEqual(editorGroupsService.enforcedOptions[0].closeEmptyGroups, false);
		});

		test('setLayout fires onDidChangeLayout event with the new layout name', async () => {
			const quadGroups = makeQuadGroups();
			const service = createService(quadGroups);

			const layoutPromise = Event.toPromise(service.onDidChangeLayout);
			await service.setLayout('quad');

			const firedLayout = await layoutPromise;
			assert.strictEqual(firedLayout, 'quad');
		});

		test('setLayout("quad") opens ForgeChatInput in all 4 groups', async () => {
			const quadGroups = makeQuadGroups();
			const service = createService(quadGroups);

			await service.setLayout('quad');

			for (const group of quadGroups) {
				assert.strictEqual(group.openedEditors.length, 1, `Group ${group.id} should have 1 editor opened`);
				assert.ok(group.openedEditors[0] instanceof ForgeChatInput, `Group ${group.id} should have a ForgeChatInput`);
			}
		});

		test('activeLayout reflects the last setLayout call', async () => {
			const quadGroups = makeQuadGroups();
			const service = createService(quadGroups);

			await service.setLayout('quad');
			assert.strictEqual(service.activeLayout, 'quad');

			// Reset groups to 1 for focus
			editorGroupsService.groups = [quadGroups[0]];
			await service.setLayout('focus');
			assert.strictEqual(service.activeLayout, 'focus');
		});

		test('setLayout with same layout as current is a no-op (no event fired)', async () => {
			const service = createService();

			const events: ForgeLayout[] = [];
			disposables.add(service.onDidChangeLayout(l => events.push(l)));

			// Already 'focus', calling focus again should be no-op
			await service.setLayout('focus');

			assert.strictEqual(events.length, 0);
		});
	});

	suite('openChatPane', () => {

		test('openChatPane maps "tl" to the stored group ID in _paneGroupMap', async () => {
			const quadGroups = makeQuadGroups();
			const service = createService(quadGroups);

			await service.setLayout('quad');

			// Clear editors opened during setLayout
			for (const g of quadGroups) { g.openedEditors.length = 0; }

			await service.openChatPane('tl', 'openai');

			assert.strictEqual(quadGroups[0].openedEditors.length, 1);
			const input = quadGroups[0].openedEditors[0] as ForgeChatInput;
			assert.ok(input instanceof ForgeChatInput);
			assert.strictEqual(input.providerName, 'openai');
		});

		test('openChatPane maps "br" to the stored group ID in _paneGroupMap', async () => {
			const quadGroups = makeQuadGroups();
			const service = createService(quadGroups);

			await service.setLayout('quad');

			for (const g of quadGroups) { g.openedEditors.length = 0; }

			await service.openChatPane('br', 'local');

			// BR is the 4th group (index 3)
			assert.strictEqual(quadGroups[3].openedEditors.length, 1);
			const input = quadGroups[3].openedEditors[0] as ForgeChatInput;
			assert.ok(input instanceof ForgeChatInput);
			assert.strictEqual(input.providerName, 'local');
		});

		test('openChatPane with providerId opens chat input with that provider', async () => {
			const quadGroups = makeQuadGroups();
			const service = createService(quadGroups);

			await service.setLayout('quad');
			for (const g of quadGroups) { g.openedEditors.length = 0; }

			await service.openChatPane('tr', 'openai');

			const input = quadGroups[1].openedEditors[0] as ForgeChatInput;
			assert.strictEqual(input.providerName, 'openai');
		});

		test('openChatPane without providerId uses default from config', async () => {
			const quadGroups = makeQuadGroups();
			const service = createService(quadGroups);

			await service.setLayout('quad');
			for (const g of quadGroups) { g.openedEditors.length = 0; }

			await service.openChatPane('tl');

			const input = quadGroups[0].openedEditors[0] as ForgeChatInput;
			assert.strictEqual(input.providerName, 'anthropic');
		});
	});

	suite('group lifecycle', () => {

		test('removing a group in quad mode triggers layout downgrade to focus', async () => {
			const quadGroups = makeQuadGroups();
			const service = createService(quadGroups);

			await service.setLayout('quad');
			assert.strictEqual(service.activeLayout, 'quad');

			const events: ForgeLayout[] = [];
			disposables.add(service.onDidChangeLayout(l => events.push(l)));

			// Simulate removing a group — now only 3 groups remain
			editorGroupsService.groups = quadGroups.slice(0, 3);
			editorGroupsService.fireDidRemoveGroup(quadGroups[3]);

			assert.strictEqual(service.activeLayout, 'focus');
			assert.strictEqual(events.length, 1);
			assert.strictEqual(events[0], 'focus');
		});

		test('enforcePartOptions disposable is cleaned up when leaving quad mode', async () => {
			const quadGroups = makeQuadGroups();
			const service = createService(quadGroups);

			await service.setLayout('quad');

			// The enforcePartOptions should have been called
			assert.strictEqual(editorGroupsService.enforcedOptions.length, 1);

			// Now switch to focus — the enforce disposable should be disposed
			editorGroupsService.groups = [quadGroups[0]];
			await service.setLayout('focus');

			// applyFocusLayout calls disposeEnforceOptions first, then applies
			// We verify indirectly: a second setLayout("quad") should call enforcePartOptions again
			editorGroupsService.groups = makeQuadGroups();
			await service.setLayout('quad');

			assert.strictEqual(editorGroupsService.enforcedOptions.length, 2);
		});

		test('_paneGroupMap is populated after setLayout("quad")', async () => {
			const quadGroups = makeQuadGroups();
			const service = createService(quadGroups);

			await service.setLayout('quad');

			const state = service.getLayoutState();
			assert.strictEqual(state.layout, 'quad');
			assert.strictEqual(state.panes.length, 4);

			// Verify all positions are present
			const positions = state.panes.map(p => p.position);
			assert.ok(positions.includes('tl'));
			assert.ok(positions.includes('tr'));
			assert.ok(positions.includes('bl'));
			assert.ok(positions.includes('br'));
		});

		test('_paneGroupMap is cleared after setLayout("focus")', async () => {
			const quadGroups = makeQuadGroups();
			const service = createService(quadGroups);

			await service.setLayout('quad');

			const quadState = service.getLayoutState();
			assert.strictEqual(quadState.panes.length, 4);

			editorGroupsService.groups = [quadGroups[0]];
			await service.setLayout('focus');

			const focusState = service.getLayoutState();
			assert.strictEqual(focusState.layout, 'focus');
			assert.strictEqual(focusState.panes.length, 0);
		});
	});

	suite('persistence', () => {

		test('saveLayout stores serialized ForgeLayoutState in workspace storage', async () => {
			const quadGroups = makeQuadGroups();
			const service = createService(quadGroups);

			await service.setLayout('quad');
			service.saveLayout();

			const raw = storageService.get('forge.layout.state', 0 /* StorageScope.PROFILE */);
			// Try workspace scope (1)
			const rawWorkspace = storageService.get('forge.layout.state', 1 /* StorageScope.WORKSPACE */);
			const stored = raw ?? rawWorkspace;
			assert.ok(stored, 'Layout state should be stored');

			const parsed = JSON.parse(stored!);
			assert.strictEqual(parsed.layout, 'quad');
			assert.strictEqual(parsed.panes.length, 4);
		});

		test('restoreLayout reads from storage and applies saved layout', async () => {
			const quadGroups = makeQuadGroups();

			// Pre-populate storage with a quad layout state
			const savedState = {
				layout: 'quad',
				panes: [
					{ position: 'tl', providerId: 'anthropic', conversationId: 'conv-1' },
					{ position: 'tr', providerId: 'openai', conversationId: 'conv-2' },
					{ position: 'bl', providerId: 'anthropic', conversationId: 'conv-3' },
					{ position: 'br', providerId: 'local', conversationId: 'conv-4' },
				],
			};
			storageService.store('forge.layout.state', JSON.stringify(savedState), 1 /* StorageScope.WORKSPACE */, 1 /* StorageTarget.MACHINE */);

			const service = createService(quadGroups);

			// restoreLayout is called automatically via whenReady, but we call explicitly to test
			await service.restoreLayout();

			assert.strictEqual(service.activeLayout, 'quad');
			const state = service.getLayoutState();
			assert.strictEqual(state.panes.length, 4);
		});

		test('restoreLayout with no saved state defaults to focus layout', async () => {
			const service = createService();

			// No storage data present
			await service.restoreLayout();

			// activeLayout should remain focus (the default)
			assert.strictEqual(service.activeLayout, 'focus');
		});

		test('restoreLayout with corrupted JSON defaults to focus layout', async () => {
			storageService.store('forge.layout.state', '{not valid json!!!', 1 /* StorageScope.WORKSPACE */, 1 /* StorageTarget.MACHINE */);

			const service = createService();

			await service.restoreLayout();

			assert.strictEqual(service.activeLayout, 'focus');
		});
	});

	suite('getLayoutState', () => {

		test('getLayoutState returns current layout and pane configurations', () => {
			const service = createService();

			const state = service.getLayoutState();

			assert.strictEqual(state.layout, 'focus');
			assert.ok(Array.isArray(state.panes));
		});

		test('getLayoutState after setLayout("quad") returns 4 pane states', async () => {
			const quadGroups = makeQuadGroups();
			const service = createService(quadGroups);

			await service.setLayout('quad');

			const state = service.getLayoutState();
			assert.strictEqual(state.layout, 'quad');
			assert.strictEqual(state.panes.length, 4);

			for (const pane of state.panes) {
				assert.ok(pane.position);
				assert.ok(pane.providerId);
				assert.ok(pane.conversationId);
			}
		});
	});
});
