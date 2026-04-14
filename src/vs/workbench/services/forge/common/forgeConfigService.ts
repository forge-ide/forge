/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { Emitter, Event } from '../../../../base/common/event.js';
import { Disposable } from '../../../../base/common/lifecycle.js';
import { IConfigurationService, ConfigurationTarget } from '../../../../platform/configuration/common/configuration.js';
import { InstantiationType, registerSingleton } from '../../../../platform/instantiation/common/extensions.js';
import { createDecorator } from '../../../../platform/instantiation/common/instantiation.js';
import { ILogService } from '../../../../platform/log/common/log.js';

import { type ForgeConfig, type ForgeProviderConfig, resolveModelConfig, type ResolvedModelConfig } from './forgeConfigTypes.js';

export type { ForgeConfig };

export const IForgeConfigService = createDecorator<IForgeConfigService>('forgeConfigService');

export interface IForgeConfigService {
	readonly _serviceBrand: undefined;
	readonly onDidChange: Event<ForgeConfig>;
	getConfig(): ForgeConfig;
	updateConfig(partial: Partial<ForgeConfig>): Promise<void>;
	resolveModel(providerName?: string, modelId?: string): ResolvedModelConfig | undefined;
	getProviders(): readonly ForgeProviderConfig[];
}

export class ForgeConfigService extends Disposable implements IForgeConfigService {

	declare readonly _serviceBrand: undefined;

	private readonly _onDidChange = this._register(new Emitter<ForgeConfig>());
	readonly onDidChange = this._onDidChange.event;

	constructor(
		@IConfigurationService private readonly configurationService: IConfigurationService,
		@ILogService private readonly logService: ILogService,
	) {
		super();

		this._register(
			this.configurationService.onDidChangeConfiguration(e => {
				if (
					e.affectsConfiguration('forge.defaultProvider') ||
					e.affectsConfiguration('forge.defaultModel') ||
					e.affectsConfiguration('forge.stream') ||
					e.affectsConfiguration('forge.providers')
				) {
					this._onDidChange.fire(this.getConfig());
				}
			})
		);
	}

	getConfig(): ForgeConfig {
		return {
			defaultProvider: this.configurationService.getValue<string>('forge.defaultProvider') ?? '',
			defaultModel: this.configurationService.getValue<string>('forge.defaultModel') ?? '',
			stream: this.configurationService.getValue<boolean>('forge.stream') ?? true,
			providers: this.configurationService.getValue<ForgeProviderConfig[]>('forge.providers') ?? [],
		};
	}

	resolveModel(providerName?: string, modelId?: string): ResolvedModelConfig | undefined {
		return resolveModelConfig(this.getConfig(), providerName, modelId);
	}

	getProviders(): readonly ForgeProviderConfig[] {
		return this.getConfig().providers;
	}

	async updateConfig(partial: Partial<ForgeConfig>): Promise<void> {
		const updates: Promise<void>[] = [];

		if (Object.prototype.hasOwnProperty.call(partial, 'defaultProvider')) {
			updates.push(this.configurationService.updateValue('forge.defaultProvider', partial.defaultProvider, ConfigurationTarget.USER));
		}
		if (Object.prototype.hasOwnProperty.call(partial, 'defaultModel')) {
			updates.push(this.configurationService.updateValue('forge.defaultModel', partial.defaultModel, ConfigurationTarget.USER));
		}
		if (Object.prototype.hasOwnProperty.call(partial, 'stream')) {
			updates.push(this.configurationService.updateValue('forge.stream', partial.stream, ConfigurationTarget.USER));
		}
		if (Object.prototype.hasOwnProperty.call(partial, 'providers')) {
			updates.push(this.configurationService.updateValue('forge.providers', partial.providers, ConfigurationTarget.USER));
		}

		if (updates.length === 0) {
			return;
		}

		try {
			await Promise.all(updates);
		} catch (error) {
			this.logService.warn(`[ForgeConfigService] Failed to update configuration: ${error}`);
			throw error;
		}
	}
}

registerSingleton(IForgeConfigService, ForgeConfigService, InstantiationType.Eager);
