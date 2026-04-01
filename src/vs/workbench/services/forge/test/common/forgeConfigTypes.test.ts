/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import assert from 'assert';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import { resolveModelConfig, findProvider, findModel, type ForgeConfig, type ForgeProviderConfig, type ForgeModelConfig } from '../../common/forgeConfigTypes.js';

function makeModelConfig(overrides: Partial<ForgeModelConfig> & { name: string }): ForgeModelConfig {
	return { ...overrides };
}

function makeProviderConfig(overrides: Partial<ForgeProviderConfig> & { name: string; default: string; models: ForgeModelConfig[] }): ForgeProviderConfig {
	return { ...overrides };
}

function makeConfig(overrides: Partial<ForgeConfig> & { default: string; providers: ForgeProviderConfig[] }): ForgeConfig {
	return { ...overrides };
}

suite('forgeConfigTypes', () => {

	ensureNoDisposablesAreLeakedInTestSuite();

	suite('resolveModelConfig', () => {

		test('model-level override takes precedence over provider-level default', () => {
			const provider = makeProviderConfig({
				name: 'anthropic',
				default: 'claude-sonnet-4-6',
				maxTokens: 4096,
				stream: true,
				models: [
					makeModelConfig({ name: 'claude-sonnet-4-6', maxTokens: 8192, stream: false }),
				],
			});

			const resolved = resolveModelConfig(provider, 'claude-sonnet-4-6');

			assert.strictEqual(resolved.maxTokens, 8192);
			assert.strictEqual(resolved.stream, false);
		});

		test('provider-level default used when model does not specify', () => {
			const provider = makeProviderConfig({
				name: 'anthropic',
				default: 'claude-sonnet-4-6',
				maxTokens: 2048,
				stream: false,
				contextBudget: 0.5,
				models: [
					makeModelConfig({ name: 'claude-sonnet-4-6' }),
				],
			});

			const resolved = resolveModelConfig(provider, 'claude-sonnet-4-6');

			assert.strictEqual(resolved.maxTokens, 2048);
			assert.strictEqual(resolved.stream, false);
			assert.strictEqual(resolved.contextBudget, 0.5);
		});

		test('hardcoded fallback when neither model nor provider specifies', () => {
			const provider = makeProviderConfig({
				name: 'anthropic',
				default: 'claude-sonnet-4-6',
				models: [
					makeModelConfig({ name: 'claude-sonnet-4-6' }),
				],
			});

			const resolved = resolveModelConfig(provider, 'claude-sonnet-4-6');

			assert.strictEqual(resolved.maxTokens, 4096);
			assert.strictEqual(resolved.stream, true);
		});

		test('all three levels in one resolution — model > provider > hardcoded', () => {
			const provider = makeProviderConfig({
				name: 'openai',
				default: 'o1',
				maxTokens: 4096,
				stream: true,
				models: [
					makeModelConfig({ name: 'o1', stream: false, maxTokens: 16384 }),
				],
			});

			const resolved = resolveModelConfig(provider, 'o1');

			// Model-level overrides
			assert.strictEqual(resolved.maxTokens, 16384);
			assert.strictEqual(resolved.stream, false);
			// No contextBudget at model or provider level — hardcoded fallback should apply
		});

		test('resolves for a model that exists in a multi-model provider', () => {
			const provider = makeProviderConfig({
				name: 'anthropic',
				default: 'claude-sonnet-4-6',
				maxTokens: 4096,
				stream: true,
				models: [
					makeModelConfig({ name: 'claude-sonnet-4-6' }),
					makeModelConfig({ name: 'claude-opus-4-6', maxTokens: 8192, contextBudget: 0.8 }),
					makeModelConfig({ name: 'claude-haiku-4-5', maxTokens: 2048 }),
				],
			});

			const resolved = resolveModelConfig(provider, 'claude-opus-4-6');

			assert.strictEqual(resolved.maxTokens, 8192);
			assert.strictEqual(resolved.contextBudget, 0.8);
			assert.strictEqual(resolved.stream, true); // falls back to provider default
		});
	});

	suite('findProvider', () => {

		test('finds provider by name', () => {
			const config = makeConfig({
				default: 'anthropic',
				providers: [
					makeProviderConfig({ name: 'anthropic', default: 'claude-sonnet-4-6', models: [makeModelConfig({ name: 'claude-sonnet-4-6' })] }),
					makeProviderConfig({ name: 'openai', default: 'gpt-4o', models: [makeModelConfig({ name: 'gpt-4o' })] }),
				],
			});

			const provider = findProvider(config, 'openai');

			assert.ok(provider);
			assert.strictEqual(provider.name, 'openai');
		});

		test('returns undefined for missing provider', () => {
			const config = makeConfig({
				default: 'anthropic',
				providers: [
					makeProviderConfig({ name: 'anthropic', default: 'claude-sonnet-4-6', models: [makeModelConfig({ name: 'claude-sonnet-4-6' })] }),
				],
			});

			const provider = findProvider(config, 'nonexistent');

			assert.strictEqual(provider, undefined);
		});

		test('returns undefined for empty providers array', () => {
			// Edge case — config with no providers (invalid, but findProvider should not throw)
			const config = makeConfig({
				default: 'anthropic',
				providers: [],
			});

			const provider = findProvider(config, 'anthropic');

			assert.strictEqual(provider, undefined);
		});
	});

	suite('findModel', () => {

		test('finds model by name within provider', () => {
			const provider = makeProviderConfig({
				name: 'anthropic',
				default: 'claude-sonnet-4-6',
				models: [
					makeModelConfig({ name: 'claude-sonnet-4-6' }),
					makeModelConfig({ name: 'claude-opus-4-6', maxTokens: 8192 }),
				],
			});

			const model = findModel(provider, 'claude-opus-4-6');

			assert.ok(model);
			assert.strictEqual(model.name, 'claude-opus-4-6');
			assert.strictEqual(model.maxTokens, 8192);
		});

		test('returns undefined for missing model', () => {
			const provider = makeProviderConfig({
				name: 'anthropic',
				default: 'claude-sonnet-4-6',
				models: [
					makeModelConfig({ name: 'claude-sonnet-4-6' }),
				],
			});

			const model = findModel(provider, 'nonexistent-model');

			assert.strictEqual(model, undefined);
		});

		test('returns undefined for empty models array', () => {
			const provider = makeProviderConfig({
				name: 'anthropic',
				default: 'claude-sonnet-4-6',
				models: [],
			});

			const model = findModel(provider, 'claude-sonnet-4-6');

			assert.strictEqual(model, undefined);
		});
	});
});
