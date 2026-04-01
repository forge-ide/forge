/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import assert from 'assert';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../base/test/common/utils.js';
import { ProviderRegistry } from '../../common/providerRegistry.js';
import type { IAIProvider, AICompletionRequest, AICompletionResponse, AIStreamChunk } from '../../common/aiProvider.js';

function makeProvider(name: string, models: string[] = ['model-a']): IAIProvider {
	return {
		name,
		availableModels: models,
		complete(_request: AICompletionRequest): Promise<AICompletionResponse> {
			return Promise.resolve({ content: '', model: models[0], inputTokens: 0, outputTokens: 0 });
		},
		async *stream(_request: AICompletionRequest): AsyncIterable<AIStreamChunk> {
			yield { delta: '', done: true };
		},
		validateCredentials() {
			return Promise.resolve({ valid: true });
		},
	};
}

suite('ProviderRegistry', () => {

	ensureNoDisposablesAreLeakedInTestSuite();

	test('register and resolve round-trip', () => {
		const registry = new ProviderRegistry();
		const provider = makeProvider('anthropic');

		registry.register('anthropic', provider);

		assert.strictEqual(registry.resolve('anthropic'), provider);
	});

	test('list returns all registered provider names', () => {
		const registry = new ProviderRegistry();

		registry.register('anthropic', makeProvider('anthropic'));
		registry.register('openai', makeProvider('openai'));
		registry.register('local', makeProvider('local'));

		assert.deepStrictEqual(registry.list(), ['anthropic', 'openai', 'local']);
	});

	test('list preserves insertion order', () => {
		const registry = new ProviderRegistry();

		registry.register('z-provider', makeProvider('z-provider'));
		registry.register('a-provider', makeProvider('a-provider'));
		registry.register('m-provider', makeProvider('m-provider'));

		assert.deepStrictEqual(registry.list(), ['z-provider', 'a-provider', 'm-provider']);
	});

	test('resolve returns undefined for unknown provider', () => {
		const registry = new ProviderRegistry();

		assert.strictEqual(registry.resolve('nonexistent'), undefined);
	});

	test('resolve returns undefined for unknown provider after other registrations', () => {
		const registry = new ProviderRegistry();
		registry.register('anthropic', makeProvider('anthropic'));

		assert.strictEqual(registry.resolve('openai'), undefined);
	});

	test('re-registering an existing name overwrites the previous provider', () => {
		const registry = new ProviderRegistry();
		const first = makeProvider('anthropic', ['model-1']);
		const second = makeProvider('anthropic', ['model-2']);

		registry.register('anthropic', first);
		assert.strictEqual(registry.resolve('anthropic'), first);

		registry.register('anthropic', second);
		assert.strictEqual(registry.resolve('anthropic'), second);
	});

	test('re-registering does not add a duplicate entry to list', () => {
		const registry = new ProviderRegistry();

		registry.register('anthropic', makeProvider('anthropic'));
		registry.register('anthropic', makeProvider('anthropic'));

		assert.deepStrictEqual(registry.list(), ['anthropic']);
	});

	test('list returns empty array when no providers are registered', () => {
		const registry = new ProviderRegistry();

		assert.deepStrictEqual(registry.list(), []);
	});

	// --- Phase 3.5: has() and unregister() ---

	test('has returns true for registered provider', () => {
		const registry = new ProviderRegistry();
		registry.register('anthropic', makeProvider('anthropic'));

		assert.strictEqual(registry.has('anthropic'), true);
	});

	test('has returns false for unregistered provider', () => {
		const registry = new ProviderRegistry();

		assert.strictEqual(registry.has('nonexistent'), false);
	});

	test('has returns false after provider is unregistered', () => {
		const registry = new ProviderRegistry();
		registry.register('anthropic', makeProvider('anthropic'));

		registry.unregister('anthropic');

		assert.strictEqual(registry.has('anthropic'), false);
	});

	test('unregister removes provider, subsequent resolve returns undefined', () => {
		const registry = new ProviderRegistry();
		const provider = makeProvider('anthropic');
		registry.register('anthropic', provider);

		registry.unregister('anthropic');

		assert.strictEqual(registry.resolve('anthropic'), undefined);
	});

	test('unregister removes provider from list', () => {
		const registry = new ProviderRegistry();
		registry.register('anthropic', makeProvider('anthropic'));
		registry.register('openai', makeProvider('openai'));

		registry.unregister('anthropic');

		assert.deepStrictEqual(registry.list(), ['openai']);
	});

	test('unregister returns false for non-existent provider', () => {
		const registry = new ProviderRegistry();

		assert.strictEqual(registry.unregister('nonexistent'), false);
	});

	test('unregister returns true for existing provider', () => {
		const registry = new ProviderRegistry();
		registry.register('anthropic', makeProvider('anthropic'));

		assert.strictEqual(registry.unregister('anthropic'), true);
	});

	test('unregister then re-register works correctly', () => {
		const registry = new ProviderRegistry();
		const first = makeProvider('anthropic', ['model-1']);
		const second = makeProvider('anthropic', ['model-2']);

		registry.register('anthropic', first);
		registry.unregister('anthropic');
		registry.register('anthropic', second);

		assert.strictEqual(registry.resolve('anthropic'), second);
		assert.deepStrictEqual(registry.list(), ['anthropic']);
	});
});
