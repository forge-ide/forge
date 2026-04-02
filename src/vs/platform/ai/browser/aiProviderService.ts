/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { Emitter } from '../../../base/common/event.js';
import { Disposable } from '../../../base/common/lifecycle.js';
import { ILogService } from '../../log/common/log.js';
import { InstantiationType, registerSingleton } from '../../instantiation/common/extensions.js';
import type { IAIProvider } from '../common/aiProvider.js';
import { IAIProviderService } from '../common/aiProviderService.js';
import { ProviderRegistry } from '../common/providerRegistry.js';

export class AIProviderService extends Disposable implements IAIProviderService {

	declare readonly _serviceBrand: undefined;

	private readonly _onDidChangeProviders = this._register(new Emitter<string[]>());
	readonly onDidChangeProviders = this._onDidChangeProviders.event;

	private readonly registry = new ProviderRegistry();
	private defaultProviderName: string | undefined;

	constructor(
		@ILogService private readonly logService: ILogService,
	) {
		super();
	}

	registerProvider(name: string, provider: IAIProvider): void {
		this.registry.register(name, provider);
		this.logService.info(`[AIProviderService] Registered provider: ${name}`);
		this._onDidChangeProviders.fire(this.registry.list());
	}

	unregisterProvider(name: string): void {
		if (this.registry.unregister(name)) {
			this.logService.info(`[AIProviderService] Unregistered provider: ${name}`);
			if (this.defaultProviderName === name) {
				this.defaultProviderName = undefined;
			}
			this._onDidChangeProviders.fire(this.registry.list());
		}
	}

	getProvider(name: string): IAIProvider | undefined {
		return this.registry.resolve(name);
	}

	has(name: string): boolean {
		return this.registry.has(name);
	}

	listProviders(): string[] {
		return this.registry.list();
	}

	getDefaultProviderName(): string | undefined {
		return this.defaultProviderName;
	}

	setDefaultProviderName(name: string): void {
		this.defaultProviderName = name;
		this.logService.info(`[AIProviderService] Default provider set to: ${name}`);
	}
}

registerSingleton(IAIProviderService, AIProviderService, InstantiationType.Delayed);
