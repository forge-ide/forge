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

	private readonly _onDidChangeProvider = this._register(new Emitter<string>());
	readonly onDidChangeProvider = this._onDidChangeProvider.event;

	private readonly registry = new ProviderRegistry();
	private activeProviderName: string | undefined;

	constructor(
		@ILogService private readonly logService: ILogService,
	) {
		super();
	}

	registerProvider(name: string, provider: IAIProvider): void {
		this.registry.register(name, provider);
		this.logService.info(`[AIProviderService] Registered provider: ${name}`);
	}

	getProvider(name: string): IAIProvider | undefined {
		return this.registry.resolve(name);
	}

	getActiveProvider(): IAIProvider | undefined {
		if (!this.activeProviderName) {
			return undefined;
		}
		return this.registry.resolve(this.activeProviderName);
	}

	setActiveProvider(name: string): void {
		this.activeProviderName = name;
		this.logService.info(`[AIProviderService] Active provider set to: ${name}`);
		this._onDidChangeProvider.fire(name);
	}

	listProviders(): string[] {
		return this.registry.list();
	}
}

registerSingleton(IAIProviderService, AIProviderService, InstantiationType.Delayed);
