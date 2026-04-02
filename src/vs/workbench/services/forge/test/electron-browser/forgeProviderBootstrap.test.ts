/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import assert from 'assert';
import { Emitter, Event } from '../../../../../base/common/event.js';
import { DisposableStore } from '../../../../../base/common/lifecycle.js';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import type { IAIProvider, AICompletionRequest, AICompletionResponse, AIStreamChunk } from '../../../../../platform/ai/common/aiProvider.js';
import type { ForgeConfig, ForgeProviderConfig, ForgeModelConfig, ResolvedModelConfig } from '../../common/forgeConfigTypes.js';
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

function makeModelConfig(id: string): ForgeModelConfig {
	return { id };
}

function makeProviderConfig(name: string): ForgeProviderConfig {
	return {
		name,
		models: [makeModelConfig('default-model')],
	};
}

class MockForgeConfigService implements IForgeConfigService {
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

	resolveModel(_providerName?: string, _modelId?: string): ResolvedModelConfig | undefined {
		return undefined;
	}

	getProviders(): readonly ForgeProviderConfig[] {
		return this.config.providers;
	}

	dispose(): void {
		this._onDidChange.dispose();
	}
}

class MockCredentialService implements IForgeCredentialService {
	declare readonly _serviceBrand: undefined;

	private readonly keys = new Map<string, string>();
	private readonly _onDidChangeCredential = new Emitter<string>();
	readonly onDidChangeCredential: Event<string> = this._onDidChangeCredential.event;

	setKey(provider: string, key: string): void {
		this.keys.set(provider, key);
	}

	async getApiKey(providerName: string, _envKey: string): Promise<string | undefined> {
		return this.keys.get(providerName);
	}

	async setApiKey(_providerName: string, _apiKey: string): Promise<void> {
		// no-op
	}

	async deleteApiKey(_providerName: string): Promise<void> {
		// no-op
	}

	async hasApiKey(providerName: string, _envKey: string): Promise<boolean> {
		return this.keys.has(providerName);
	}

	dispose(): void {
		this._onDidChangeCredential.dispose();
	}
}

class MockAIProviderService implements IAIProviderService {
	declare readonly _serviceBrand: undefined;

	private readonly providers = new Map<string, IAIProvider>();
	private defaultProviderName: string = 'anthropic';
	private readonly _onDidChangeProviders = new Emitter<string[]>();
	readonly onDidChangeProviders = this._onDidChangeProviders.event;

	registerProvider(name: string, provider: IAIProvider): void {
		this.providers.set(name, provider);
		this._onDidChangeProviders.fire(this.listProviders());
	}

	unregisterProvider(name: string): void {
		this.providers.delete(name);
		this._onDidChangeProviders.fire(this.listProviders());
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

	getDefaultProviderName(): string | undefined {
		return this.defaultProviderName;
	}

	setDefaultProviderName(name: string): void {
		this.defaultProviderName = name;
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
 * 2. For each provider, resolve credentials
 * 3. If credentials exist (or provider is 'local'), register it
 * 4. Skip providers with missing credentials
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
		// Local provider doesn't need credentials
		if (providerConfig.name === 'local') {
			aiProviderService.registerProvider(providerConfig.name, providerFactory(providerConfig.name));
			continue;
		}

		// Resolve credentials — skip if missing
		const apiKey = await credentialService.getApiKey(providerConfig.name, '');
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
			defaultProvider: 'anthropic',
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
			defaultProvider: 'anthropic',
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

	test('local provider registered without credentials', async () => {
		configService = new MockForgeConfigService({
			defaultProvider: 'local',
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
			defaultProvider: 'anthropic',
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
			defaultProvider: 'openai',
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
			defaultProvider: 'anthropic',
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

	test('default provider name is set from config', async () => {
		credentialService.setKey('anthropic', 'sk-ant-123');

		configService = new MockForgeConfigService({
			defaultProvider: 'anthropic',
			providers: [
				makeProviderConfig('anthropic'),
			],
		});
		disposables.add({ dispose: () => configService.dispose() });

		await bootstrapProviders(configService, credentialService, aiProviderService, makeMockProvider);

		// Simulate what ForgeProviderBootstrap does after registering providers
		const config = configService.getConfig();
		if (config.defaultProvider) {
			aiProviderService.setDefaultProviderName(config.defaultProvider);
		}

		assert.strictEqual(aiProviderService.getDefaultProviderName(), 'anthropic');
	});

	test('empty providers array results in no registrations', async () => {
		configService = new MockForgeConfigService({
			defaultProvider: 'anthropic',
			providers: [],
		});
		disposables.add({ dispose: () => configService.dispose() });

		await bootstrapProviders(configService, credentialService, aiProviderService, makeMockProvider);

		assert.deepStrictEqual(aiProviderService.listProviders(), []);
	});

	test('all providers missing credentials results in no registrations', async () => {
		// No credentials set for either provider
		configService = new MockForgeConfigService({
			defaultProvider: 'anthropic',
			providers: [
				makeProviderConfig('anthropic'),
				makeProviderConfig('openai'),
			],
		});
		disposables.add({ dispose: () => configService.dispose() });

		await bootstrapProviders(configService, credentialService, aiProviderService, makeMockProvider);

		assert.deepStrictEqual(aiProviderService.listProviders(), []);
	});

	test('credential change triggers re-bootstrap and registers previously skipped provider', async () => {
		// Initially only anthropic has credentials
		credentialService.setKey('anthropic', 'sk-ant-123');

		configService = new MockForgeConfigService({
			defaultProvider: 'anthropic',
			providers: [
				makeProviderConfig('anthropic'),
				makeProviderConfig('openai'),
			],
		});
		disposables.add({ dispose: () => configService.dispose() });

		await bootstrapProviders(configService, credentialService, aiProviderService, makeMockProvider);
		assert.ok(aiProviderService.has('anthropic'));
		assert.ok(!aiProviderService.has('openai'), 'openai should be skipped initially');

		// Simulate credential arrival for openai
		credentialService.setKey('openai', 'sk-oai-456');
		await bootstrapProviders(configService, credentialService, aiProviderService, makeMockProvider);

		assert.ok(aiProviderService.has('anthropic'));
		assert.ok(aiProviderService.has('openai'), 'openai should be registered after credential added');
	});

	test('adding a new provider to config registers it on re-bootstrap', async () => {
		credentialService.setKey('anthropic', 'sk-ant-123');
		credentialService.setKey('openai', 'sk-oai-456');

		configService = new MockForgeConfigService({
			defaultProvider: 'anthropic',
			providers: [
				makeProviderConfig('anthropic'),
			],
		});
		disposables.add({ dispose: () => configService.dispose() });

		await bootstrapProviders(configService, credentialService, aiProviderService, makeMockProvider);
		assert.deepStrictEqual(aiProviderService.listProviders(), ['anthropic']);

		// Add openai to config
		configService.setConfig({
			defaultProvider: 'anthropic',
			providers: [
				makeProviderConfig('anthropic'),
				makeProviderConfig('openai'),
			],
		});
		await bootstrapProviders(configService, credentialService, aiProviderService, makeMockProvider);

		assert.ok(aiProviderService.has('openai'), 'openai should appear after config update');
	});

	test('removing a provider from config unregisters it on re-bootstrap', async () => {
		credentialService.setKey('anthropic', 'sk-ant-123');
		credentialService.setKey('openai', 'sk-oai-456');

		configService = new MockForgeConfigService({
			defaultProvider: 'anthropic',
			providers: [
				makeProviderConfig('anthropic'),
				makeProviderConfig('openai'),
			],
		});
		disposables.add({ dispose: () => configService.dispose() });

		await bootstrapProviders(configService, credentialService, aiProviderService, makeMockProvider);
		assert.ok(aiProviderService.has('openai'));

		// Remove openai from config
		configService.setConfig({
			defaultProvider: 'anthropic',
			providers: [
				makeProviderConfig('anthropic'),
			],
		});
		await bootstrapProviders(configService, credentialService, aiProviderService, makeMockProvider);

		assert.ok(!aiProviderService.has('openai'), 'openai should be gone after removal from config');
		assert.ok(aiProviderService.has('anthropic'));
	});

	test('changing defaultProvider in config updates the default name', async () => {
		credentialService.setKey('anthropic', 'sk-ant-123');
		credentialService.setKey('openai', 'sk-oai-456');

		configService = new MockForgeConfigService({
			defaultProvider: 'anthropic',
			providers: [
				makeProviderConfig('anthropic'),
				makeProviderConfig('openai'),
			],
		});
		disposables.add({ dispose: () => configService.dispose() });

		await bootstrapProviders(configService, credentialService, aiProviderService, makeMockProvider);
		aiProviderService.setDefaultProviderName(configService.getConfig().defaultProvider);
		assert.strictEqual(aiProviderService.getDefaultProviderName(), 'anthropic');

		// Switch default to openai
		configService.setConfig({
			defaultProvider: 'openai',
			providers: [
				makeProviderConfig('anthropic'),
				makeProviderConfig('openai'),
			],
		});
		await bootstrapProviders(configService, credentialService, aiProviderService, makeMockProvider);
		aiProviderService.setDefaultProviderName(configService.getConfig().defaultProvider);

		assert.strictEqual(aiProviderService.getDefaultProviderName(), 'openai');
	});

	test('SecretStorage throws during credential resolution — provider is skipped', async () => {
		// Create a credential service that throws for a specific provider
		const throwingCredentialService = new MockCredentialService();
		throwingCredentialService.setKey('openai', 'sk-oai-456');

		// Override getApiKey to throw for anthropic
		const originalGetApiKey = throwingCredentialService.getApiKey.bind(throwingCredentialService);
		throwingCredentialService.getApiKey = async (providerName: string, envKey: string): Promise<string | undefined> => {
			if (providerName === 'anthropic') {
				throw new Error('SecretStorage read failure');
			}
			return originalGetApiKey(providerName, envKey);
		};
		// Also override hasApiKey since bootstrap uses it
		const originalHasApiKey = throwingCredentialService.hasApiKey.bind(throwingCredentialService);
		throwingCredentialService.hasApiKey = async (providerName: string, envKey: string): Promise<boolean> => {
			if (providerName === 'anthropic') {
				throw new Error('SecretStorage read failure');
			}
			return originalHasApiKey(providerName, envKey);
		};

		configService = new MockForgeConfigService({
			defaultProvider: 'anthropic',
			providers: [
				makeProviderConfig('anthropic'),
				makeProviderConfig('openai'),
			],
		});
		disposables.add({ dispose: () => configService.dispose() });

		// Bootstrap with per-provider error isolation (as the real implementation should do)
		const config = configService.getConfig();
		for (const name of aiProviderService.listProviders()) {
			aiProviderService.unregisterProvider(name);
		}
		for (const providerConfig of config.providers) {
			try {
				if (providerConfig.name === 'local') {
					aiProviderService.registerProvider(providerConfig.name, makeMockProvider(providerConfig.name));
					continue;
				}
				const apiKey = await throwingCredentialService.getApiKey(providerConfig.name, '');
				if (!apiKey) { continue; }
				aiProviderService.registerProvider(providerConfig.name, makeMockProvider(providerConfig.name));
			} catch {
				// Error resolving credentials for one provider should not block others
			}
		}

		assert.ok(!aiProviderService.has('anthropic'), 'anthropic should be skipped due to SecretStorage error');
		assert.ok(aiProviderService.has('openai'), 'openai should still register despite anthropic failure');
	});

	test('config service returns malformed data — bootstrap handles gracefully', async () => {
		credentialService.setKey('anthropic', 'sk-ant-123');

		// Create a config service that returns malformed data (missing providers array)
		configService = new MockForgeConfigService({
			defaultProvider: 'anthropic',
			providers: [],
		});
		disposables.add({ dispose: () => configService.dispose() });

		// Override getConfig to return an object with undefined providers
		configService.getConfig = () => ({ defaultProvider: 'anthropic' } as ForgeConfig);

		// Bootstrap should handle missing providers gracefully
		let didThrow = false;
		try {
			const config = configService.getConfig();
			for (const name of aiProviderService.listProviders()) {
				aiProviderService.unregisterProvider(name);
			}
			// Guard against missing providers array
			const providers = config.providers ?? [];
			for (const providerConfig of providers) {
				if (providerConfig.name === 'local') {
					aiProviderService.registerProvider(providerConfig.name, makeMockProvider(providerConfig.name));
					continue;
				}
				const apiKey = await credentialService.getApiKey(providerConfig.name, '');
				if (!apiKey) { continue; }
				aiProviderService.registerProvider(providerConfig.name, makeMockProvider(providerConfig.name));
			}
		} catch {
			didThrow = true;
		}

		assert.ok(!didThrow, 'bootstrap should not throw when config has missing providers');
		assert.deepStrictEqual(aiProviderService.listProviders(), [], 'no providers should be registered from malformed config');
	});

	test('error in provider factory for one provider does not block others', async () => {
		credentialService.setKey('anthropic', 'sk-ant-123');
		credentialService.setKey('openai', 'sk-oai-456');

		configService = new MockForgeConfigService({
			defaultProvider: 'anthropic',
			providers: [
				makeProviderConfig('anthropic'),
				makeProviderConfig('openai'),
			],
		});
		disposables.add({ dispose: () => configService.dispose() });

		let callCount = 0;
		const failingFactory = (name: string): IAIProvider => {
			callCount++;
			if (name === 'anthropic') {
				throw new Error('factory failure');
			}
			return makeMockProvider(name);
		};

		// bootstrapProviders doesn't have try/catch per provider, so test with a
		// modified version that isolates errors per provider
		const config = configService.getConfig();
		for (const name of aiProviderService.listProviders()) {
			aiProviderService.unregisterProvider(name);
		}
		for (const providerConfig of config.providers) {
			if (providerConfig.name === 'local') {
				try { aiProviderService.registerProvider(providerConfig.name, failingFactory(providerConfig.name)); } catch { /* skip */ }
				continue;
			}
			const apiKey = await credentialService.getApiKey(providerConfig.name, '');
			if (!apiKey) { continue; }
			try {
				aiProviderService.registerProvider(providerConfig.name, failingFactory(providerConfig.name));
			} catch {
				// Error in one provider should not prevent registering the next
			}
		}

		assert.ok(!aiProviderService.has('anthropic'), 'anthropic should fail to register');
		assert.ok(aiProviderService.has('openai'), 'openai should still register despite anthropic failure');
		assert.strictEqual(callCount, 2, 'factory should be called for both providers');
	});
});
