/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { VSBuffer } from '../../../../base/common/buffer.js';
import { Emitter, Event } from '../../../../base/common/event.js';
import { Disposable } from '../../../../base/common/lifecycle.js';
import { joinPath } from '../../../../base/common/resources.js';
import { URI } from '../../../../base/common/uri.js';
import { IEnvironmentService } from '../../../../platform/environment/common/environment.js';
import { IFileService } from '../../../../platform/files/common/files.js';
import { InstantiationType, registerSingleton } from '../../../../platform/instantiation/common/extensions.js';
import { createDecorator } from '../../../../platform/instantiation/common/instantiation.js';
import { ILogService } from '../../../../platform/log/common/log.js';
import { IWorkspaceContextService } from '../../../../platform/workspace/common/workspace.js';

const CONFIG_FILENAME = 'forge.json';

const DEFAULT_CONFIG: ForgeConfig = {
	provider: 'anthropic',
	model: 'claude-sonnet-4-6',
};

export interface ForgeConfig {
	provider: string;
	model?: string;
	maxTokens?: number;
	stream?: boolean;
	providers?: Record<string, { model?: string; baseURL?: string }>;
}

export const IForgeConfigService = createDecorator<IForgeConfigService>('forgeConfigService');

export interface IForgeConfigService {
	readonly _serviceBrand: undefined;
	readonly onDidChange: Event<ForgeConfig>;
	getConfig(): ForgeConfig;
	updateConfig(partial: Partial<ForgeConfig>): Promise<void>;
}

export class ForgeConfigService extends Disposable implements IForgeConfigService {

	declare readonly _serviceBrand: undefined;

	private readonly _onDidChange = this._register(new Emitter<ForgeConfig>());
	readonly onDidChange = this._onDidChange.event;

	private config: ForgeConfig = { ...DEFAULT_CONFIG };

	private readonly workspaceConfigUri: URI | undefined;
	private readonly globalConfigUri: URI;

	constructor(
		@IFileService private readonly fileService: IFileService,
		@IWorkspaceContextService private readonly workspaceContextService: IWorkspaceContextService,
		@ILogService private readonly logService: ILogService,
		@IEnvironmentService environmentService: IEnvironmentService,
	) {
		super();

		// Workspace-level config: first workspace folder root
		const folders = this.workspaceContextService.getWorkspace().folders;
		if (folders.length > 0) {
			this.workspaceConfigUri = joinPath(folders[0].uri, CONFIG_FILENAME);
		}

		// Global config: <userRoamingDataHome>/forge/forge.json
		this.globalConfigUri = joinPath(environmentService.userRoamingDataHome, 'forge', CONFIG_FILENAME);

		this.setupFileWatching();
		this.loadConfig();
	}

	private setupFileWatching(): void {
		// Watch workspace config if available
		if (this.workspaceConfigUri) {
			this._register(this.fileService.watch(this.workspaceConfigUri));
		}
		this._register(this.fileService.watch(this.globalConfigUri));

		// Reload only when forge.json files change (debounced)
		this._register(
			Event.debounce(
				Event.filter(this.fileService.onDidFilesChange, e =>
					(this.workspaceConfigUri !== undefined && e.affects(this.workspaceConfigUri))
					|| e.affects(this.globalConfigUri)
				),
				() => undefined,
				100,
			)(() => {
				this.loadConfig();
			})
		);
	}

	private loadConfig(): void {
		this.loadConfigAsync().catch(error => {
			this.logService.warn('[ForgeConfigService] Failed to load config, using defaults', error);
		});
	}

	private async loadConfigAsync(): Promise<void> {
		// Try workspace config first, then global, then defaults
		const parsed = await this.tryReadConfigFile(this.workspaceConfigUri)
			?? await this.tryReadConfigFile(this.globalConfigUri);

		const newConfig: ForgeConfig = {
			...DEFAULT_CONFIG,
			...(parsed ?? {}),
		};

		const changed = JSON.stringify(newConfig) !== JSON.stringify(this.config);
		this.config = newConfig;
		if (changed) {
			this._onDidChange.fire(this.config);
		}
	}

	private async tryReadConfigFile(uri: URI | undefined): Promise<Partial<ForgeConfig> | undefined> {
		if (!uri) {
			return undefined;
		}

		let text: string;
		try {
			const content = await this.fileService.readFile(uri);
			text = content.value.toString();
		} catch {
			// File doesn't exist or is unreadable — expected
			return undefined;
		}

		try {
			return JSON.parse(text) as Partial<ForgeConfig>;
		} catch (parseError) {
			this.logService.warn(`[ForgeConfigService] Invalid JSON in ${uri.path}`, parseError);
			return undefined;
		}
	}

	getConfig(): ForgeConfig {
		return { ...this.config };
	}

	async updateConfig(partial: Partial<ForgeConfig>): Promise<void> {
		this.config = { ...this.config, ...partial };
		this._onDidChange.fire(this.config);

		// Persist to workspace config if available, otherwise global
		const targetUri = this.workspaceConfigUri ?? this.globalConfigUri;
		try {
			const serialized = JSON.stringify(this.config, undefined, '\t');
			await this.fileService.writeFile(targetUri, VSBuffer.fromString(serialized));
		} catch (error) {
			this.logService.warn('[ForgeConfigService] Failed to write config file', error);
		}
	}
}

registerSingleton(IForgeConfigService, ForgeConfigService, InstantiationType.Eager);
