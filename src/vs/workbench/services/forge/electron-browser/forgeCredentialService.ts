/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { Emitter } from '../../../../base/common/event.js';
import { Disposable } from '../../../../base/common/lifecycle.js';
import { process } from '../../../../base/parts/sandbox/electron-browser/globals.js';
import { ILogService } from '../../../../platform/log/common/log.js';
import { InstantiationType, registerSingleton } from '../../../../platform/instantiation/common/extensions.js';
import { ISecretStorageService } from '../../../../platform/secrets/common/secrets.js';
import { IForgeCredentialService } from '../common/forgeCredentialService.js';

const SECRET_PREFIX = 'forge.provider.';

export class ForgeCredentialService extends Disposable implements IForgeCredentialService {

	declare readonly _serviceBrand: undefined;

	private readonly _onDidChangeCredential = this._register(new Emitter<string>());
	readonly onDidChangeCredential = this._onDidChangeCredential.event;

	constructor(
		@ISecretStorageService private readonly secretStorage: ISecretStorageService,
		@ILogService private readonly logService: ILogService,
	) {
		super();

		this._register(this.secretStorage.onDidChangeSecret(key => {
			if (key.startsWith(SECRET_PREFIX)) {
				const providerName = key.slice(SECRET_PREFIX.length);
				this._onDidChangeCredential.fire(providerName);
			}
		}));
	}

	async getApiKey(providerName: string, envKey: string): Promise<string | undefined> {
		// SecretStorage takes priority over environment variables
		const stored = await this.secretStorage.get(SECRET_PREFIX + providerName);
		if (stored) {
			this.logService.trace(`[ForgeCredentialService] Retrieved stored key for '${providerName}'`);
			return stored;
		}

		// Fall back to environment variable
		const envValue = process.env[envKey];
		if (envValue) {
			this.logService.trace(`[ForgeCredentialService] Using env var '${envKey}' for '${providerName}'`);
			return envValue;
		}

		this.logService.debug(`[ForgeCredentialService] No credential found for '${providerName}' (env: ${envKey})`);
		return undefined;
	}

	async setApiKey(providerName: string, apiKey: string): Promise<void> {
		await this.secretStorage.set(SECRET_PREFIX + providerName, apiKey);
		this.logService.info(`[ForgeCredentialService] Stored API key for '${providerName}'`);
	}

	async deleteApiKey(providerName: string): Promise<void> {
		await this.secretStorage.delete(SECRET_PREFIX + providerName);
		this.logService.info(`[ForgeCredentialService] Deleted API key for '${providerName}'`);
	}

	async hasApiKey(providerName: string, envKey: string): Promise<boolean> {
		const stored = await this.secretStorage.get(SECRET_PREFIX + providerName);
		if (stored) {
			return true;
		}
		return !!process.env[envKey];
	}
}

registerSingleton(IForgeCredentialService, ForgeCredentialService, InstantiationType.Delayed);
