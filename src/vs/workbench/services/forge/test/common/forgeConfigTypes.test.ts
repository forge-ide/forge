/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import assert from 'assert';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import { resolveModelConfig, findProvider, findModel, type ForgeConfig, type ForgeProviderConfig, type ForgeModelConfig } from '../../common/forgeConfigTypes.js';

function makeModelConfig(overrides: Partial<ForgeModelConfig> & { id: string }): ForgeModelConfig {
	return { ...overrides };
}

function makeProviderConfig(overrides: Partial<ForgeProviderConfig> & { name: string; models: ForgeModelConfig[] }): ForgeProviderConfig {
	return { ...overrides };
}

function makeConfig(overrides: Partial<ForgeConfig> & { defaultProvider: string; providers: ForgeProviderConfig[] }): ForgeConfig {
	return { ...overrides };
}

suite('forgeConfigTypes', () => {

	ensureNoDisposablesAreLeakedInTestSuite();

	suite('resolveModelConfig', () => {

		test('model-level override takes precedence over hardcoded default', () => {
			const config = makeConfig({
				defaultProvider: 'anthropic',
				defaultModel: 'claude-sonnet-4-6',
				stream: true,
				providers: [
					makeProviderConfig({
						name: 'anthropic',
						models: [
							makeModelConfig({ id: 'claude-sonnet-4-6', maxTokens: 8192 }),
						],
					}),
				],
			});

			const resolved = resolveModelConfig(config, 'anthropic', 'claude-sonnet-4-6');

			assert.ok(resolved);
			assert.strictEqual(resolved.maxTokens, 8192);
			assert.strictEqual(resolved.stream, true);
		});

		test('hardcoded fallback when model does not specify maxTokens', () => {
			const config = makeConfig({
				defaultProvider: 'anthropic',
				defaultModel: 'claude-sonnet-4-6',
				providers: [
					makeProviderConfig({
						name: 'anthropic',
						models: [
							makeModelConfig({ id: 'claude-sonnet-4-6' }),
						],
					}),
				],
			});

			const resolved = resolveModelConfig(config, 'anthropic', 'claude-sonnet-4-6');

			assert.ok(resolved);
			assert.strictEqual(resolved.maxTokens, 4096);
			assert.strictEqual(resolved.stream, true);
		});

		test('stream defaults to true when not specified', () => {
			const config = makeConfig({
				defaultProvider: 'anthropic',
				providers: [
					makeProviderConfig({
						name: 'anthropic',
						models: [
							makeModelConfig({ id: 'claude-sonnet-4-6' }),
						],
					}),
				],
			});

			const resolved = resolveModelConfig(config);

			assert.ok(resolved);
			assert.strictEqual(resolved.stream, true);
		});

		test('config-level stream: false overrides the default', () => {
			const config = makeConfig({
				defaultProvider: 'openai',
				stream: false,
				providers: [
					makeProviderConfig({
						name: 'openai',
						models: [
							makeModelConfig({ id: 'o1', maxTokens: 16384 }),
						],
					}),
				],
			});

			const resolved = resolveModelConfig(config, 'openai', 'o1');

			assert.ok(resolved);
			assert.strictEqual(resolved.maxTokens, 16384);
			assert.strictEqual(resolved.stream, false);
		});

		test('resolves for a model that exists in a multi-model provider', () => {
			const config = makeConfig({
				defaultProvider: 'anthropic',
				defaultModel: 'claude-sonnet-4-6',
				stream: true,
				providers: [
					makeProviderConfig({
						name: 'anthropic',
						models: [
							makeModelConfig({ id: 'claude-sonnet-4-6' }),
							makeModelConfig({ id: 'claude-opus-4-6', maxTokens: 8192, contextBudget: 16000 }),
							makeModelConfig({ id: 'claude-haiku-4-5', maxTokens: 2048 }),
						],
					}),
				],
			});

			const resolved = resolveModelConfig(config, 'anthropic', 'claude-opus-4-6');

			assert.ok(resolved);
			assert.strictEqual(resolved.maxTokens, 8192);
			assert.strictEqual(resolved.contextBudget, 16000);
			assert.strictEqual(resolved.stream, true);
		});

		test('returns undefined when resolved model not found in provider', () => {
			const config = makeConfig({
				defaultProvider: 'anthropic',
				defaultModel: 'claude-sonnet-4-6',
				providers: [
					makeProviderConfig({
						name: 'openai',
						models: [makeModelConfig({ id: 'gpt-4o' })],
					}),
				],
			});

			// defaultModel is 'claude-sonnet-4-6' but openai has no such model;
			// provider.models[0] is 'gpt-4o' so that takes priority
			const resolved = resolveModelConfig(config, 'openai');
			assert.ok(resolved);
			assert.strictEqual(resolved.modelId, 'gpt-4o');
		});

		test('returns undefined when explicit modelId not in provider', () => {
			const config = makeConfig({
				defaultProvider: 'anthropic',
				providers: [
					makeProviderConfig({
						name: 'anthropic',
						models: [makeModelConfig({ id: 'claude-sonnet-4-6' })],
					}),
				],
			});

			const resolved = resolveModelConfig(config, 'anthropic', 'nonexistent-model');
			assert.strictEqual(resolved, undefined);
		});

		test('returns undefined for unknown provider', () => {
			const config = makeConfig({
				defaultProvider: 'anthropic',
				providers: [
					makeProviderConfig({
						name: 'anthropic',
						models: [makeModelConfig({ id: 'claude-sonnet-4-6' })],
					}),
				],
			});

			const resolved = resolveModelConfig(config, 'nonexistent');

			assert.strictEqual(resolved, undefined);
		});

		test('uses defaultProvider when providerName not specified', () => {
			const config = makeConfig({
				defaultProvider: 'anthropic',
				defaultModel: 'claude-sonnet-4-6',
				providers: [
					makeProviderConfig({
						name: 'anthropic',
						models: [makeModelConfig({ id: 'claude-sonnet-4-6' })],
					}),
				],
			});

			const resolved = resolveModelConfig(config);

			assert.ok(resolved);
			assert.strictEqual(resolved.providerName, 'anthropic');
			assert.strictEqual(resolved.modelId, 'claude-sonnet-4-6');
		});
	});

	suite('resolveModelConfig — config expansion', () => {

		test('model-level contextBudget override', () => {
			const config = makeConfig({
				defaultProvider: 'anthropic',
				providers: [
					makeProviderConfig({
						name: 'anthropic',
						models: [
							makeModelConfig({ id: 'claude-sonnet-4-6', contextBudget: 16000 }),
						],
					}),
				],
			});

			const resolved = resolveModelConfig(config, 'anthropic', 'claude-sonnet-4-6');

			assert.ok(resolved);
			assert.strictEqual(resolved.contextBudget, 16000);
			assert.strictEqual(resolved.maxTokens, 4096); // default preserved
		});

		test('custom envKey on provider is used in resolved config', () => {
			const config = makeConfig({
				defaultProvider: 'anthropic',
				providers: [
					makeProviderConfig({
						name: 'anthropic',
						envKey: 'MY_CUSTOM_KEY',
						models: [makeModelConfig({ id: 'claude-sonnet-4-6' })],
					}),
				],
			});

			const resolved = resolveModelConfig(config, 'anthropic', 'claude-sonnet-4-6');

			assert.ok(resolved);
			assert.strictEqual(resolved.envKey, 'MY_CUSTOM_KEY');
		});

		test('default envKey from PROVIDER_ENV_VARS when provider has no envKey', () => {
			const config = makeConfig({
				defaultProvider: 'anthropic',
				providers: [
					makeProviderConfig({
						name: 'anthropic',
						models: [makeModelConfig({ id: 'claude-sonnet-4-6' })],
					}),
				],
			});

			const resolved = resolveModelConfig(config, 'anthropic', 'claude-sonnet-4-6');

			assert.ok(resolved);
			assert.strictEqual(resolved.envKey, 'ANTHROPIC_API_KEY');
		});

		test('custom baseURL on provider is passed through to resolved config', () => {
			const config = makeConfig({
				defaultProvider: 'openai',
				providers: [
					makeProviderConfig({
						name: 'openai',
						baseURL: 'https://proxy.example.com',
						models: [makeModelConfig({ id: 'gpt-4o' })],
					}),
				],
			});

			const resolved = resolveModelConfig(config, 'openai', 'gpt-4o');

			assert.ok(resolved);
			assert.strictEqual(resolved.baseURL, 'https://proxy.example.com');
		});

		test('unknown provider with no PROVIDER_ENV_VARS entry generates fallback envKey', () => {
			const config = makeConfig({
				defaultProvider: 'custom-llm',
				providers: [
					makeProviderConfig({
						name: 'custom-llm',
						models: [makeModelConfig({ id: 'custom-model' })],
					}),
				],
			});

			const resolved = resolveModelConfig(config, 'custom-llm', 'custom-model');

			assert.ok(resolved);
			assert.strictEqual(resolved.envKey, 'CUSTOM-LLM_API_KEY');
		});

		test('multi-provider config resolves each provider independently', () => {
			const config = makeConfig({
				defaultProvider: 'anthropic',
				providers: [
					makeProviderConfig({
						name: 'anthropic',
						models: [makeModelConfig({ id: 'claude-sonnet-4-6', maxTokens: 8192 })],
					}),
					makeProviderConfig({
						name: 'openai',
						baseURL: 'https://api.openai.com',
						models: [makeModelConfig({ id: 'gpt-4o', maxTokens: 16384 })],
					}),
					makeProviderConfig({
						name: 'local',
						models: [makeModelConfig({ id: 'llama-3' })],
					}),
				],
			});

			const anthropic = resolveModelConfig(config, 'anthropic', 'claude-sonnet-4-6');
			const openai = resolveModelConfig(config, 'openai', 'gpt-4o');
			const local = resolveModelConfig(config, 'local', 'llama-3');

			assert.ok(anthropic);
			assert.strictEqual(anthropic.maxTokens, 8192);
			assert.strictEqual(anthropic.envKey, 'ANTHROPIC_API_KEY');

			assert.ok(openai);
			assert.strictEqual(openai.maxTokens, 16384);
			assert.strictEqual(openai.baseURL, 'https://api.openai.com');

			assert.ok(local);
			assert.strictEqual(local.maxTokens, 4096); // default
		});
	});

	suite('findProvider', () => {

		test('finds provider by name', () => {
			const config = makeConfig({
				defaultProvider: 'anthropic',
				providers: [
					makeProviderConfig({ name: 'anthropic', models: [makeModelConfig({ id: 'claude-sonnet-4-6' })] }),
					makeProviderConfig({ name: 'openai', models: [makeModelConfig({ id: 'gpt-4o' })] }),
				],
			});

			const provider = findProvider(config, 'openai');

			assert.ok(provider);
			assert.strictEqual(provider.name, 'openai');
		});

		test('returns undefined for missing provider', () => {
			const config = makeConfig({
				defaultProvider: 'anthropic',
				providers: [
					makeProviderConfig({ name: 'anthropic', models: [makeModelConfig({ id: 'claude-sonnet-4-6' })] }),
				],
			});

			const provider = findProvider(config, 'nonexistent');

			assert.strictEqual(provider, undefined);
		});

		test('returns undefined for empty providers array', () => {
			const config = makeConfig({
				defaultProvider: 'anthropic',
				providers: [],
			});

			const provider = findProvider(config, 'anthropic');

			assert.strictEqual(provider, undefined);
		});

		test('lookup is case-sensitive', () => {
			const config = makeConfig({
				defaultProvider: 'anthropic',
				providers: [
					makeProviderConfig({ name: 'anthropic', models: [makeModelConfig({ id: 'claude-sonnet-4-6' })] }),
				],
			});

			assert.strictEqual(findProvider(config, 'Anthropic'), undefined);
			assert.strictEqual(findProvider(config, 'ANTHROPIC'), undefined);
		});
	});

	suite('findModel', () => {

		test('finds model by id within provider', () => {
			const provider = makeProviderConfig({
				name: 'anthropic',
				models: [
					makeModelConfig({ id: 'claude-sonnet-4-6' }),
					makeModelConfig({ id: 'claude-opus-4-6', maxTokens: 8192 }),
				],
			});

			const model = findModel(provider, 'claude-opus-4-6');

			assert.ok(model);
			assert.strictEqual(model.id, 'claude-opus-4-6');
			assert.strictEqual(model.maxTokens, 8192);
		});

		test('returns undefined for missing model', () => {
			const provider = makeProviderConfig({
				name: 'anthropic',
				models: [
					makeModelConfig({ id: 'claude-sonnet-4-6' }),
				],
			});

			const model = findModel(provider, 'nonexistent-model');

			assert.strictEqual(model, undefined);
		});

		test('returns undefined for empty models array', () => {
			const provider = makeProviderConfig({
				name: 'anthropic',
				models: [],
			});

			const model = findModel(provider, 'claude-sonnet-4-6');

			assert.strictEqual(model, undefined);
		});
	});
});
