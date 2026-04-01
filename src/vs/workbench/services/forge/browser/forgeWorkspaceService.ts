/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { Emitter } from '../../../../base/common/event.js';
import { Disposable } from '../../../../base/common/lifecycle.js';
import { generateUuid } from '../../../../base/common/uuid.js';
import { ILogService } from '../../../../platform/log/common/log.js';
import { InstantiationType, registerSingleton } from '../../../../platform/instantiation/common/extensions.js';
import { IStorageService, StorageScope, StorageTarget } from '../../../../platform/storage/common/storage.js';
import { IForgeLayoutService } from '../common/forgeLayoutService.js';
import { IForgeWorkspaceService } from '../common/forgeWorkspaceService.js';
import type { ForgeWorkspaceConfig } from '../common/forgeWorkspaceTypes.js';

const WORKSPACES_STORAGE_KEY = 'forge.workspaces';
const ACTIVE_WORKSPACE_STORAGE_KEY = 'forge.activeWorkspaceId';

export class ForgeWorkspaceService extends Disposable implements IForgeWorkspaceService {

	declare readonly _serviceBrand: undefined;

	private readonly _onDidChangeActiveWorkspace = this._register(new Emitter<ForgeWorkspaceConfig | undefined>());
	readonly onDidChangeActiveWorkspace = this._onDidChangeActiveWorkspace.event;

	private readonly _onDidChangeWorkspaces = this._register(new Emitter<void>());
	readonly onDidChangeWorkspaces = this._onDidChangeWorkspaces.event;

	private workspaces: ForgeWorkspaceConfig[] = [];
	private activeWorkspaceId: string | undefined;

	constructor(
		@IStorageService private readonly storageService: IStorageService,
		@IForgeLayoutService private readonly forgeLayoutService: IForgeLayoutService,
		@ILogService private readonly logService: ILogService,
	) {
		super();

		this.loadWorkspaces();
		this.activeWorkspaceId = this.storageService.get(ACTIVE_WORKSPACE_STORAGE_KEY, StorageScope.WORKSPACE);

		this.registerPersistenceHooks();
	}

	getWorkspaces(): ForgeWorkspaceConfig[] {
		return [...this.workspaces];
	}

	getActiveWorkspace(): ForgeWorkspaceConfig | undefined {
		if (!this.activeWorkspaceId) {
			return undefined;
		}
		return this.workspaces.find(w => w.id === this.activeWorkspaceId);
	}

	async createWorkspace(name: string): Promise<ForgeWorkspaceConfig> {
		const layoutState = this.forgeLayoutService.getLayoutState();

		const workspace: ForgeWorkspaceConfig = {
			id: generateUuid(),
			name,
			createdAt: Date.now(),
			layout: layoutState.layout,
			panes: layoutState.panes,
			conversations: [],
		};

		this.workspaces.push(workspace);
		this.activeWorkspaceId = workspace.id;

		this.persistWorkspaces();
		this.persistActiveWorkspaceId();

		this.logService.info(`[ForgeWorkspaceService] Created workspace '${name}' (${workspace.id})`);

		this._onDidChangeWorkspaces.fire();
		this._onDidChangeActiveWorkspace.fire(workspace);

		return workspace;
	}

	async saveActiveWorkspace(): Promise<void> {
		const active = this.getActiveWorkspace();
		if (!active) {
			this.logService.warn('[ForgeWorkspaceService] No active workspace to save');
			return;
		}

		const layoutState = this.forgeLayoutService.getLayoutState();

		const updated: ForgeWorkspaceConfig = {
			...active,
			layout: layoutState.layout,
			panes: layoutState.panes,
		};

		const index = this.workspaces.findIndex(w => w.id === active.id);
		if (index !== -1) {
			this.workspaces[index] = updated;
		}

		this.persistWorkspaces();

		this.logService.info(`[ForgeWorkspaceService] Saved workspace '${active.name}' (${active.id})`);

		this._onDidChangeWorkspaces.fire();
		this._onDidChangeActiveWorkspace.fire(updated);
	}

	async switchWorkspace(id: string): Promise<void> {
		const workspace = this.workspaces.find(w => w.id === id);
		if (!workspace) {
			this.logService.warn(`[ForgeWorkspaceService] Workspace '${id}' not found`);
			return;
		}

		this.activeWorkspaceId = workspace.id;
		this.persistActiveWorkspaceId();

		// Restore the layout
		await this.forgeLayoutService.setLayout(workspace.layout);

		// Open panes for the workspace
		for (const pane of workspace.panes) {
			await this.forgeLayoutService.openChatPane(pane.position, pane.providerId);
		}

		this.logService.info(`[ForgeWorkspaceService] Switched to workspace '${workspace.name}' (${workspace.id})`);

		this._onDidChangeActiveWorkspace.fire(workspace);
	}

	async deleteWorkspace(id: string): Promise<void> {
		const index = this.workspaces.findIndex(w => w.id === id);
		if (index === -1) {
			this.logService.warn(`[ForgeWorkspaceService] Workspace '${id}' not found for deletion`);
			return;
		}

		const workspace = this.workspaces[index];
		this.workspaces.splice(index, 1);

		if (this.activeWorkspaceId === id) {
			this.activeWorkspaceId = undefined;
			this.persistActiveWorkspaceId();
			this._onDidChangeActiveWorkspace.fire(undefined);
		}

		this.persistWorkspaces();

		this.logService.info(`[ForgeWorkspaceService] Deleted workspace '${workspace.name}' (${id})`);

		this._onDidChangeWorkspaces.fire();
	}

	async renameWorkspace(id: string, newName: string): Promise<void> {
		const index = this.workspaces.findIndex(w => w.id === id);
		if (index === -1) {
			this.logService.warn(`[ForgeWorkspaceService] Workspace '${id}' not found for rename`);
			return;
		}

		const updated: ForgeWorkspaceConfig = {
			...this.workspaces[index],
			name: newName,
		};
		this.workspaces[index] = updated;

		this.persistWorkspaces();

		this.logService.info(`[ForgeWorkspaceService] Renamed workspace '${id}' to '${newName}'`);

		this._onDidChangeWorkspaces.fire();

		if (this.activeWorkspaceId === id) {
			this._onDidChangeActiveWorkspace.fire(updated);
		}
	}

	// --- Private: Persistence ---

	private loadWorkspaces(): void {
		const raw = this.storageService.get(WORKSPACES_STORAGE_KEY, StorageScope.PROFILE);
		if (!raw) {
			this.workspaces = [];
			return;
		}

		try {
			this.workspaces = JSON.parse(raw) as ForgeWorkspaceConfig[];
		} catch (error) {
			this.logService.warn('[ForgeWorkspaceService] Failed to parse stored workspaces', error);
			this.workspaces = [];
		}
	}

	private persistWorkspaces(): void {
		try {
			const json = JSON.stringify(this.workspaces);
			this.storageService.store(WORKSPACES_STORAGE_KEY, json, StorageScope.PROFILE, StorageTarget.USER);
		} catch (error) {
			this.logService.warn('[ForgeWorkspaceService] Failed to persist workspaces', error);
		}
	}

	private persistActiveWorkspaceId(): void {
		if (this.activeWorkspaceId) {
			this.storageService.store(ACTIVE_WORKSPACE_STORAGE_KEY, this.activeWorkspaceId, StorageScope.WORKSPACE, StorageTarget.MACHINE);
		} else {
			this.storageService.remove(ACTIVE_WORKSPACE_STORAGE_KEY, StorageScope.WORKSPACE);
		}
	}

	private registerPersistenceHooks(): void {
		this._register(this.storageService.onWillSaveState(() => {
			this.persistWorkspaces();
		}));
	}
}

registerSingleton(IForgeWorkspaceService, ForgeWorkspaceService, InstantiationType.Delayed);
