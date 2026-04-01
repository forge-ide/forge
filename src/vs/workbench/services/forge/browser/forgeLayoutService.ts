/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { Emitter } from '../../../../base/common/event.js';
import { Disposable, IDisposable } from '../../../../base/common/lifecycle.js';
import { generateUuid } from '../../../../base/common/uuid.js';
import { ILogService } from '../../../../platform/log/common/log.js';
import { InstantiationType, registerSingleton } from '../../../../platform/instantiation/common/extensions.js';
import { IStorageService, StorageScope, StorageTarget } from '../../../../platform/storage/common/storage.js';
import { ForgeChatInput } from '../../../browser/parts/editor/forgeChat/forgeChatInput.js';
import { GroupIdentifier } from '../../../common/editor.js';
import { GroupOrientation, GroupsOrder, IEditorGroup, IEditorGroupsService, MergeGroupMode } from '../../editor/common/editorGroupsService.js';
import { IForgeConfigService } from '../common/forgeConfigService.js';
import { ForgeLayout, ForgeLayoutState, ForgePaneState, IForgeLayoutService, PanePosition } from '../common/forgeLayoutService.js';

const STORAGE_KEY = 'forge.layout.state';

const PANE_POSITIONS: readonly PanePosition[] = ['tl', 'tr', 'bl', 'br'];

export class ForgeLayoutService extends Disposable implements IForgeLayoutService {

	declare readonly _serviceBrand: undefined;

	private readonly _onDidChangeLayout = this._register(new Emitter<ForgeLayout>());
	readonly onDidChangeLayout = this._onDidChangeLayout.event;

	private _activeLayout: ForgeLayout = 'focus';
	get activeLayout(): ForgeLayout { return this._activeLayout; }

	private readonly _paneGroupMap = new Map<PanePosition, GroupIdentifier>();
	private readonly _paneStateMap = new Map<PanePosition, ForgePaneState>();

	private _enforceOptionsDisposable: IDisposable | undefined;

	constructor(
		@IEditorGroupsService private readonly editorGroupsService: IEditorGroupsService,
		@IStorageService private readonly storageService: IStorageService,
		@IForgeConfigService private readonly forgeConfigService: IForgeConfigService,
		@ILogService private readonly logService: ILogService,
	) {
		super();

		this.registerGroupLifecycleListeners();
		this.registerPersistenceHooks();

		this.editorGroupsService.whenReady.then(() => {
			this.restoreLayout().catch(error => {
				this.logService.warn('[ForgeLayoutService] Failed to restore layout', error);
			});
		});
	}

	async setLayout(layout: ForgeLayout): Promise<void> {
		if (layout === this._activeLayout) {
			return;
		}

		this.logService.info(`[ForgeLayoutService] Switching layout to '${layout}'`);

		switch (layout) {
			case 'focus':
				await this.applyFocusLayout();
				break;
			case 'split':
				await this.applySplitLayout();
				break;
			case 'quad':
				await this.applyQuadLayout();
				break;
			case 'code+ai':
				await this.applyCodeAiLayout();
				break;
		}

		this._activeLayout = layout;
		this._onDidChangeLayout.fire(layout);
	}

	async openChatPane(position: PanePosition, providerId?: string): Promise<void> {
		await this._openChatPane(position, providerId);
	}

	private async _openChatPane(position: PanePosition, providerId?: string): Promise<void> {
		if (this._activeLayout !== 'quad') {
			await this.setLayout('quad');
		}

		const groupId = this._paneGroupMap.get(position);
		if (groupId === undefined) {
			this.logService.warn(`[ForgeLayoutService] No group mapped for position '${position}'`);
			return;
		}

		const group = this.editorGroupsService.getGroup(groupId);
		if (!group) {
			this.logService.warn(`[ForgeLayoutService] Group ${groupId} not found for position '${position}'`);
			return;
		}

		const resolvedProvider = providerId ?? this.forgeConfigService.getConfig().defaultProvider;
		const conversationId = generateUuid();
		const chatInput = new ForgeChatInput(resolvedProvider, conversationId);

		this._paneStateMap.set(position, {
			position,
			providerId: resolvedProvider,
			conversationId,
		});

		try {
			await group.closeAllEditors();
			await group.openEditor(chatInput);
		} catch (error) {
			chatInput.dispose();
			throw error;
		}
	}

	getLayoutState(): ForgeLayoutState {
		return {
			layout: this._activeLayout,
			panes: Array.from(this._paneStateMap.values()),
		};
	}

	saveLayout(): void {
		const state = this.getLayoutState();
		try {
			const json = JSON.stringify(state);
			this.storageService.store(STORAGE_KEY, json, StorageScope.WORKSPACE, StorageTarget.MACHINE);
			this.logService.debug('[ForgeLayoutService] Layout state saved');
		} catch (error) {
			this.logService.warn('[ForgeLayoutService] Failed to save layout state', error);
		}
	}

	async restoreLayout(): Promise<void> {
		const raw = this.storageService.get(STORAGE_KEY, StorageScope.WORKSPACE);
		if (!raw) {
			this.logService.debug('[ForgeLayoutService] No saved layout state found');
			return;
		}

		let saved: ForgeLayoutState;
		try {
			saved = JSON.parse(raw) as ForgeLayoutState;
		} catch (error) {
			this.logService.warn('[ForgeLayoutService] Invalid saved layout state', error);
			return;
		}

		// Do NOT re-apply the grid layout -- VS Code handles grid persistence.
		// Verify the current grid shape matches what we saved.
		const groups = this.editorGroupsService.getGroups(GroupsOrder.GRID_APPEARANCE);

		if (saved.layout === 'quad' && groups.length === 4) {
			this._activeLayout = 'quad';
			this.mapGroupsToPositions(groups);
			this.restorePaneStates(saved.panes);
		} else if (saved.layout === 'split' && groups.length === 2) {
			this._activeLayout = 'split';
			this._paneGroupMap.clear();
		} else if (saved.layout === 'code+ai' && groups.length === 2) {
			this._activeLayout = 'code+ai';
			this._paneGroupMap.clear();
		} else if (saved.layout === 'focus' && groups.length === 1) {
			this._activeLayout = 'focus';
			this._paneGroupMap.clear();
		} else {
			// Grid shape diverges from saved state -- fall back to focus
			this.logService.info('[ForgeLayoutService] Grid shape does not match saved layout, falling back to focus');
			this._activeLayout = 'focus';
			this._paneGroupMap.clear();
		}

		this._onDidChangeLayout.fire(this._activeLayout);
	}

	// --- Private: Layout application ---

	private async applyFocusLayout(): Promise<void> {
		this.disposeEnforceOptions();
		await this.closeAllChatEditors();

		// Move all editors from non-active groups into the active group
		const activeGroup = this.editorGroupsService.activeGroup;
		const groups = this.editorGroupsService.getGroups(GroupsOrder.CREATION_TIME);

		for (const group of groups) {
			if (group.id !== activeGroup.id) {
				this.editorGroupsService.mergeGroup(group, activeGroup, { mode: MergeGroupMode.MOVE_EDITORS });
			}
		}

		this.editorGroupsService.mainPart.applyLayout({
			orientation: GroupOrientation.HORIZONTAL,
			groups: [{}],
		});

		this._paneGroupMap.clear();
		this._paneStateMap.clear();
	}

	private async applySplitLayout(): Promise<void> {
		this.disposeEnforceOptions();
		await this.closeAllChatEditors();

		// Prevent empty groups from being auto-closed when dragging the last tab out
		this._enforceOptionsDisposable = this.editorGroupsService.mainPart.enforcePartOptions({
			closeEmptyGroups: false,
		});

		this.editorGroupsService.mainPart.applyLayout({
			orientation: GroupOrientation.HORIZONTAL,
			groups: [{}, {}],
		});

		this._paneGroupMap.clear();
		this._paneStateMap.clear();
	}

	private async applyQuadLayout(): Promise<void> {
		this.disposeEnforceOptions();
		await this.closeAllChatEditors();

		// Prevent empty groups from being auto-closed; hide tab bar so welcome screens are uncluttered
		this._enforceOptionsDisposable = this.editorGroupsService.mainPart.enforcePartOptions({
			closeEmptyGroups: false,
			showTabs: 'none',
		});

		this.editorGroupsService.mainPart.applyLayout({
			orientation: GroupOrientation.HORIZONTAL,
			groups: [
				{ groups: [{}, {}] }, // top row: TL, TR
				{ groups: [{}, {}] }, // bottom row: BL, BR
			],
		});

		// Capture groups immediately after layout and map to positions
		const groups = this.editorGroupsService.getGroups(GroupsOrder.GRID_APPEARANCE);
		if (groups.length !== 4) {
			this.logService.error(`[ForgeLayoutService] Expected 4 groups after quad layout, got ${groups.length}`);
			return;
		}

		this.mapGroupsToPositions(groups);
	}

	private async applyCodeAiLayout(): Promise<void> {
		this.disposeEnforceOptions();
		await this.closeAllChatEditors();

		// Prevent empty groups from being auto-closed when dragging the last tab out
		this._enforceOptionsDisposable = this.editorGroupsService.mainPart.enforcePartOptions({
			closeEmptyGroups: false,
		});

		this.editorGroupsService.mainPart.applyLayout({
			orientation: GroupOrientation.HORIZONTAL,
			groups: [{}, {}],
		});

		// Open a chat pane in the right group only
		const groups = this.editorGroupsService.getGroups(GroupsOrder.GRID_APPEARANCE);
		if (groups.length !== 2) {
			this.logService.error(`[ForgeLayoutService] Expected 2 groups after code+ai layout, got ${groups.length}`);
			return;
		}

		const rightGroup = groups[1];
		const config = this.forgeConfigService.getConfig();
		const conversationId = generateUuid();
		const chatInput = new ForgeChatInput(config.defaultProvider, conversationId);

		this._paneGroupMap.clear();
		this._paneStateMap.clear();
		this._paneStateMap.set('tr', {
			position: 'tr',
			providerId: config.defaultProvider,
			conversationId,
		});

		try {
			await rightGroup.openEditor(chatInput);
		} catch (error) {
			chatInput.dispose();
			throw error;
		}
	}

	// --- Private: Helpers ---

	private async closeAllChatEditors(): Promise<void> {
		const groups = this.editorGroupsService.getGroups(GroupsOrder.CREATION_TIME);
		for (const group of groups) {
			const editorsToClose = group.editors.filter(e =>
				e.typeId === ForgeChatInput.ID && !(e as ForgeChatInput).hasHistory
			);
			if (editorsToClose.length > 0) {
				await group.closeEditors(editorsToClose);
			}
		}
	}

	private mapGroupsToPositions(groups: readonly IEditorGroup[]): void {
		this._paneGroupMap.clear();
		// Groups are in grid appearance order: TL=0, TR=1, BL=2, BR=3
		for (let i = 0; i < Math.min(groups.length, PANE_POSITIONS.length); i++) {
			this._paneGroupMap.set(PANE_POSITIONS[i], groups[i].id);
		}
	}

	private restorePaneStates(panes: ForgePaneState[]): void {
		this._paneStateMap.clear();
		for (const pane of panes) {
			this._paneStateMap.set(pane.position, pane);
		}
	}

	private disposeEnforceOptions(): void {
		if (this._enforceOptionsDisposable) {
			this._enforceOptionsDisposable.dispose();
			this._enforceOptionsDisposable = undefined;
		}
	}

	private registerGroupLifecycleListeners(): void {
		this._register(this.editorGroupsService.onDidRemoveGroup(() => {
			this.validateGridShape();
		}));

		this._register(this.editorGroupsService.onDidAddGroup(() => {
			this.validateGridShape();
		}));
	}

	private validateGridShape(): void {
		const groups = this.editorGroupsService.getGroups(GroupsOrder.GRID_APPEARANCE);
		const expectedCount = this.getExpectedGroupCount(this._activeLayout);

		if (groups.length !== expectedCount) {
			this.logService.debug(
				`[ForgeLayoutService] Grid shape diverged: expected ${expectedCount} groups for '${this._activeLayout}', found ${groups.length}`
			);

			// Downgrade to focus if grid no longer matches
			if (this._activeLayout !== 'focus') {
				this._activeLayout = 'focus';
				this._paneGroupMap.clear();
				this._onDidChangeLayout.fire(this._activeLayout);
			}
		}
	}

	private getExpectedGroupCount(layout: ForgeLayout): number {
		switch (layout) {
			case 'focus': return 1;
			case 'split': return 2;
			case 'quad': return 4;
			case 'code+ai': return 2;
		}
	}

	private registerPersistenceHooks(): void {
		this._register(this.storageService.onWillSaveState(() => {
			this.saveLayout();
		}));
	}

	override dispose(): void {
		this.disposeEnforceOptions();
		super.dispose();
	}
}

registerSingleton(IForgeLayoutService, ForgeLayoutService, InstantiationType.Delayed);
