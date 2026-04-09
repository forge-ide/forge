/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { Disposable } from '../../../../base/common/lifecycle.js';
import { ILogService } from '../../../../platform/log/common/log.js';
import { IAIProviderService } from '../../../../platform/ai/common/aiProviderService.js';
import { IForgeConfigService } from '../common/forgeConfigService.js';
import { IForgeCredentialService } from '../common/forgeCredentialService.js';
import { resolveModelConfig, type ForgeProviderConfig } from '../common/forgeConfigTypes.js';
import { WorkbenchPhase, registerWorkbenchContribution2 } from '../../../common/contributions.js';

/** Minimal shape of AnthropicVertex client needed to construct VertexProvider. */
interface IAnthropicVertexClientShape {
	messages: {
		stream(params: unknown): AsyncIterable<Record<string, unknown>>;
		create(params: unknown): Promise<unknown>;
	};
}

/**
 * Workbench contribution that bootstraps AI providers from forge.json config
 * after the workbench has been restored. Reads provider configs, resolves
 * credentials, and sets the default provider on IAIProviderService.
 */
export class ForgeProviderBootstrap extends Disposable {

	static readonly ID = 'workbench.contrib.forgeProviderBootstrap';

	private _bootstrapPromise: Promise<void> | undefined;

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
		if (this._bootstrapPromise) {
			return; // already in flight
		}
		this._bootstrapPromise = this.bootstrapAsync().finally(() => {
			this._bootstrapPromise = undefined;
		});
		this._bootstrapPromise.catch(error => {
			this.logService.error('[ForgeProviderBootstrap] Bootstrap failed', error);
		});
	}

	private async bootstrapAsync(): Promise<void> {
		const config = this.forgeConfigService.getConfig();

		for (const providerConfig of config.providers) {
			try {
				await this._registerProvider(providerConfig);
			} catch (err) {
				this.logService.error(`[ForgeProviderBootstrap] Failed to register '${providerConfig.name}'`, err);
			}
		}

		if (config.defaultProvider) {
			this.aiProviderService.setDefaultProviderName(config.defaultProvider);
		}
	}

	private async _registerProvider(providerConfig: ForgeProviderConfig): Promise<void> {
		const { name } = providerConfig;

		if (name === 'vertex') {
			await this._registerVertex(providerConfig);
			return;
		}

		const resolved = resolveModelConfig(this.forgeConfigService.getConfig(), name);
		if (!resolved) { return; }

		const hasKey = await this.credentialService.hasApiKey(name, resolved.envKey);
		if (hasKey) {
			this.logService.info(`[ForgeProviderBootstrap] Credential available for '${name}'`);
		} else {
			this.logService.debug(`[ForgeProviderBootstrap] No credential for '${name}', skipping`);
		}
	}

	private async _registerVertex(providerConfig: ForgeProviderConfig): Promise<void> {
		const projectId = providerConfig.projectId ?? process.env['GOOGLE_CLOUD_PROJECT'];
		const location = providerConfig.location ?? process.env['GOOGLE_CLOUD_LOCATION'];

		if (!projectId || !location) {
			this.logService.warn('[ForgeProviderBootstrap] Vertex: missing projectId or location, skipping registration');
			return;
		}

		const serviceAccountJson = await this.credentialService.getApiKey('vertex', '');

		let parsedCredentials: Record<string, unknown> | undefined;
		if (serviceAccountJson) {
			try {
				parsedCredentials = JSON.parse(serviceAccountJson) as Record<string, unknown>;
			} catch (e) {
				this.logService.warn('[ForgeProviderBootstrap] Vertex: failed to parse service account JSON from SecretStorage, falling back to ADC');
				parsedCredentials = undefined;
			}
		}

		const authOptions = parsedCredentials
			? { googleAuthOptions: { credentials: parsedCredentials } }
			: {};

		const models = providerConfig.models.map(m => m.id);

		const { GoogleGenAI } = await import('@google/genai');

		const ai = new GoogleGenAI({ vertexai: true, project: projectId, location, ...authOptions });

		const { AnthropicVertex } = await import('@anthropic-ai/vertex-sdk');

		let anthropicClient: IAnthropicVertexClientShape;
		if (parsedCredentials) {
			const { GoogleAuth } = await import('google-auth-library');
			const googleAuth = new GoogleAuth({
				credentials: parsedCredentials as Record<string, unknown>,
				scopes: ['https://www.googleapis.com/auth/cloud-platform'],
			}) as unknown as ConstructorParameters<typeof AnthropicVertex>[0] extends { googleAuth?: infer G } ? NonNullable<G> : never;
			anthropicClient = new AnthropicVertex({ projectId, region: location, googleAuth }) as unknown as IAnthropicVertexClientShape;
		} else {
			anthropicClient = new AnthropicVertex({ projectId, region: location }) as unknown as IAnthropicVertexClientShape;
		}

		const { VertexProvider } = await import('../../../../platform/ai/node/vertexProvider.js');
		const provider = new VertexProvider(
			ai.models as unknown as ConstructorParameters<typeof VertexProvider>[0],
			anthropicClient as unknown as ConstructorParameters<typeof VertexProvider>[1],
			models.length ? models : undefined,
		);

		this.aiProviderService.registerProvider('vertex', provider);
		this.logService.info('[ForgeProviderBootstrap] Registered vertex provider');
	}
}

registerWorkbenchContribution2(ForgeProviderBootstrap.ID, ForgeProviderBootstrap, WorkbenchPhase.AfterRestored);
