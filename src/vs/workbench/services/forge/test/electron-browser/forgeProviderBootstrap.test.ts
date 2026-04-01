/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import assert from 'assert';
import { Emitter, Event } from '../../../../../base/common/event.js';
import { DisposableStore } from '../../../../../base/common/lifecycle.js';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import { NullLogService } from '../../../../../platform/log/common/log.js';
import type { IAIProvider, AICompletionRequest, AICompletionResponse, AIStreamChunk } from '../../../../../platform/ai/common/aiProvider.js';
import type { ForgeConfig, ForgeProviderConfig, ForgeModelConfig } from '../../common/forgeConfigTypes.js';
import type { IForgeConfigService } from '../../common/forgeConfigService.js';
import type { IForgeCredentialService } from '../../common/forgeCredentialService.js';
import type { IAIProviderService } from '../../../../../platform/ai/common/aiProviderService.js';

// --- Mocks ---

function makeMockProvider(name: string): IAIProvider {
	return {
		name,
		availableModels: ['mock-model'],
		complete(_request: AICompletionRequest): Promise<AICompletionResponse> {
			return Promise.resolve({ content: '', model: 'mock-model', inputTokens: 0, outputTokens: 0 });
		},
		async *stream(_request: AICompletionRequest): AsyncIterable<AIStreamChunk> {
			yield { delta: '', done: true };
		},
		validateCredentials() {
			return Promise.resolve({ valid: true });
		},
	};
}

function makeModelConfig(name: string): ForgeModelConfig {
	return { name };
}

function makeProviderConfig(name: string, opts?: { enabled?: boolean }): ForgeProviderConfig {
	return {
		name,
		default: 'default-model',
		models: [makeModelConfig('default-model')],
		enabled: opts?.enabled,
	};
}

class MockForgeConfigService implements Partial<IForgeConfigService> {
	declare readonly _serviceBrand: undefined;

	private config: ForgeConfig;
	private readonly _onDidChange = new Emitter<ForgeConfig>();
	readonly onDidChange = this._onDidChange.event;

	constructor(config: ForgeConfig) {
		this.config = config;
	}

	getConfig(): ForgeConfig {
		return { ...this.config };
	}

	setConfig(config: ForgeConfig): void {
		this.config = config;
		this._onDidChange.fire(config);
	}

	async updateConfig(): Promise<void> {
		// no-op for tests
	}

	dispose(): void {
		this._onDidChange.dispose();
	}
}

class MockCredentialService implements Partial<IForgeCredentialService> {
	declare readonly _serviceBrand: undefined;

	private readonly keys = new Map<string, string>();

	setKey(provider: string, key: string): void {
		this.keys.set(provider, key);
	}

	async resolveApiKey(providerName: string): Promise<string | undefined> {
		return this.keys.get(providerName);
	}

	async storeApiKey(_providerName: string, _apiKey: string): Promise<void> {
		// no-op
	}

	async deleteApiKey(_providerName: string): Promise<void> {
		// no-op
	}

	resolveBaseURL(_providerName: string): string | undefined {
		return undefined;
	}
}

class MockAIProviderService implements Partial<IAIProviderService> {
	declare readonly _serviceBrand: undefined;

	private readonly providers = new Map<string, IAIProvider>();
	private readonly _onDidChangeProviders = new Emitter<void>();
	readonly onDidChangeProviders = this._onDidChangeProviders.event;

	registerProvider(name: string, provider: IAIProvider): void {
		this.providers.set(name, provider);
		this._onDidChangeProviders.fire();
	}

	unregisterProvider(name: string): void {
		this.providers.delete(name);
		this._onDidChangeProviders.fire();
	}

	getProvider(name: string): IAIProvider | undefined {
		return this.providers.get(name);
	}

	has(name: string): boolean {
		return this.providers.has(name);
	}

	listProviders(): string[] {
		return Array.from(this.providers.keys());
	}

	getDefaultProviderName(): string {
		return 'anthropic';
	}

	dispose(): void {
		this._onDidChangeProviders.dispose();
	}
}

/**
 * Minimal bootstrap function that mirrors the logic described in the Phase 3.5 plan.
 * The actual ForgeProviderBootstrap is a workbench contribution — we test its core logic here.
 *
 * This function simulates what the bootstrap contribution does:
 * 1. Read config
 * 2. For each enabled provider, resolve credentials
 * 3. If credentials exist (or provider is 'local'), register it
 * 4. Skip providers with missing credentials or enabled: false
 */
async function bootstrapProviders(
	configService: MockForgeConfigService,
	credentialService: MockCredentialService,
	aiProviderService: MockAIProviderService,
	providerFactory: (name: string) => IAIProvider,
): Promise<void> {
	const config = configService.getConfig();

	// Unregister all existing providers first
	for (const name of aiProviderService.listProviders()) {
		aiProviderService.unregisterProvider(name);
	}

	for (const providerConfig of config.providers) {
		// Skip disabled providers
		if (providerConfig.enabled === false) {
			continue;
		}

		// Local provider doesn't need credentials
		if (providerConfig.name === 'local') {
			aiProviderService.registerProvider(providerConfig.name, providerFactory(providerConfig.name));
			continue;
		}

		// Resolve credentials — skip if missing
		const apiKey = await credentialService.resolveApiKey(providerConfig.name);
		if (!apiKey) {
			continue;
		}

		aiProviderService.registerProvider(providerConfig.name, providerFactory(providerConfig.name));
	}
}

suite('ForgeProviderBootstrap', () => {

	let disposables: DisposableStore;
	let configService: MockForgeConfigService;
	let credentialService: MockCredentialService;
	let aiProviderService: MockAIProviderService;

	setup(() => {
		disposables = new DisposableStore();
		credentialService = new MockCredentialService();
		aiProviderService = new MockAIProviderService();
	});

	teardown(() => {
		disposables.dispose();
	});

	ensureNoDisposablesAreLeakedInTestSuite();

	test('providers from config are registered with AIProviderService', async () => {
		credentialService.setKey('anthropic', 'sk-ant-123');
		credentialService.setKey('openai', 'sk-oai-456');

		configService = new MockForgeConfigService({
			default: 'anthropic',
			providers: [
				makeProviderConfig('anthropic'),
				makeProviderConfig('openai'),
			],
		});
		disposables.add({ dispose: () => configService.dispose() });

		await bootstrapProviders(configService, credentialService, aiProviderService, makeMockProvider);

		assert.ok(aiProviderService.has('anthropic'), 'anthropic should be registered');
		assert.ok(aiProviderService.has('openai'), 'openai should be registered');
		assert.deepStrictEqual(aiProviderService.listProviders().sort(), ['anthropic', 'openai']);
	});

	test('provider with missing credentials is skipped', async () => {
		credentialService.setKey('anthropic', 'sk-ant-123');
		// openai has no credentials

		configService = new MockForgeConfigService({
			default: 'anthropic',
			providers: [
				makeProviderConfig('anthropic'),
				makeProviderConfig('openai'),
			],
		});
		disposables.add({ dispose: () => configService.dispose() });

		await bootstrapProviders(configService, credentialService, aiProviderService, makeMockProvider);

		assert.ok(aiProviderService.has('anthropic'), 'anthropic should be registered');
		assert.ok(!aiProviderService.has('openai'), 'openai should be skipped (no credentials)');
	});

	test('provider with enabled: false is skipped', async () => {
		credentialService.setKey('anthropic', 'sk-ant-123');
		credentialService.setKey('gemini', 'gemini-key');

		configService = new MockForgeConfigService({
			default: 'anthropic',
			providers: [
				makeProviderConfig('anthropic'),
				makeProviderConfig('gemini', { enabled: false }),
			],
		});
		disposables.add({ dispose: () => configService.dispose() });

		await bootstrapProviders(configService, credentialService, aiProviderService, makeMockProvider);

		assert.ok(aiProviderService.has('anthropic'), 'anthropic should be registered');
		assert.ok(!aiProviderService.has('gemini'), 'gemini should be skipped (enabled: false)');
	});

	test('local provider registered without credentials', async () => {
		configService = new MockForgeConfigService({
			default: 'local',
			providers: [
				makeProviderConfig('local'),
			],
		});
		disposables.add({ dispose: () => configService.dispose() });

		await bootstrapProviders(configService, credentialService, aiProviderService, makeMockProvider);

		assert.ok(aiProviderService.has('local'), 'local should be registered without credentials');
	});

	test('config change triggers re-bootstrap — old providers unregistered, new ones registered', async () => {
		credentialService.setKey('anthropic', 'sk-ant-123');
		credentialService.setKey('openai', 'sk-oai-456');

		configService = new MockForgeConfigService({
			default: 'anthropic',
			providers: [
				makeProviderConfig('anthropic'),
			],
		});
		disposables.add({ dispose: () => configService.dispose() });

		// Initial bootstrap
		await bootstrapProviders(configService, credentialService, aiProviderService, makeMockProvider);
		assert.ok(aiProviderService.has('anthropic'));
		assert.ok(!aiProviderService.has('openai'));

		// Simulate config change — now includes openai, removes anthropic
		configService.setConfig({
			default: 'openai',
			providers: [
				makeProviderConfig('openai'),
			],
		});

		// Re-bootstrap (in real code, this is triggered by onDidChange listener)
		await bootstrapProviders(configService, credentialService, aiProviderService, makeMockProvider);

		assert.ok(!aiProviderService.has('anthropic'), 'anthropic should be unregistered after config change');
		assert.ok(aiProviderService.has('openai'), 'openai should be registered after config change');
	});

	test('re-bootstrap replaces existing providers with fresh instances', async () => {
		credentialService.setKey('anthropic', 'sk-ant-123');

		configService = new MockForgeConfigService({
			default: 'anthropic',
			providers: [
				makeProviderConfig('anthropic'),
			],
		});
		disposables.add({ dispose: () => configService.dispose() });

		await bootstrapProviders(configService, credentialService, aiProviderService, makeMockProvider);
		const firstInstance = aiProviderService.getProvider('anthropic');

		// Re-bootstrap with same config — should get a new instance
		await bootstrapProviders(configService, credentialService, aiProviderService, makeMockProvider);
		const secondInstance = aiProviderService.getProvider('anthropic');

		assert.notStrictEqual(firstInstance, secondInstance, 'provider instance should be replaced on re-bootstrap');
	});
});
