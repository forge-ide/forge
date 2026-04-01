/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { Disposable } from '../../../../base/common/lifecycle.js';
import { ILogService } from '../../../../platform/log/common/log.js';
import { IAIProviderService } from '../../../../platform/ai/common/aiProviderService.js';
import { IForgeConfigService } from '../common/forgeConfigService.js';
import { IForgeCredentialService } from '../common/forgeCredentialService.js';
import { resolveModelConfig } from '../common/forgeConfigTypes.js';
import { WorkbenchPhase, registerWorkbenchContribution2 } from '../../../common/contributions.js';

/**
 * Workbench contribution that bootstraps AI providers from forge.json config
 * after the workbench has been restored. Reads provider configs, resolves
 * credentials, and sets the default provider on IAIProviderService.
 */
export class ForgeProviderBootstrap extends Disposable {

	static readonly ID = 'workbench.contrib.forgeProviderBootstrap';

	constructor(
		@IAIProviderService private readonly aiProviderService: IAIProviderService,
		@IForgeConfigService private readonly forgeConfigService: IForgeConfigService,
		@IForgeCredentialService private readonly credentialService: IForgeCredentialService,
		@ILogService private readonly logService: ILogService,
	) {
		super();

		this.bootstrap();

		this._register(this.forgeConfigService.onDidChange(() => {
			this.bootstrap();
		}));

		this._register(this.credentialService.onDidChangeCredential(() => {
			this.bootstrap();
		}));
	}

	private bootstrap(): void {
		this.bootstrapAsync().catch(error => {
			this.logService.error('[ForgeProviderBootstrap] Bootstrap failed', error);
		});
	}

	private async bootstrapAsync(): Promise<void> {
		const config = this.forgeConfigService.getConfig();

		for (const providerConfig of config.providers) {
			const resolved = resolveModelConfig(config, providerConfig.name);
			if (!resolved) {
				continue;
			}

			const hasKey = await this.credentialService.hasApiKey(providerConfig.name, resolved.envKey);
			if (hasKey) {
				this.logService.info(`[ForgeProviderBootstrap] Credential available for '${providerConfig.name}'`);
			} else {
				this.logService.debug(`[ForgeProviderBootstrap] No credential for '${providerConfig.name}', skipping`);
			}
		}

		// Set the default provider name from config
		if (config.defaultProvider) {
			this.aiProviderService.setDefaultProviderName(config.defaultProvider);
		}
	}
}

registerWorkbenchContribution2(ForgeProviderBootstrap.ID, ForgeProviderBootstrap, WorkbenchPhase.AfterRestored);
