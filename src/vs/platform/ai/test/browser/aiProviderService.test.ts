/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import assert from 'assert';
import { DisposableStore } from '../../../../base/common/lifecycle.js';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../base/test/common/utils.js';
import { IAIProvider, AICompletionRequest, AICompletionResponse, AIStreamChunk } from '../../common/aiProvider.js';
import { AIProviderService } from '../../browser/aiProviderService.js';
import { NullLogService } from '../../../log/common/log.js';

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

suite('AIProviderService', () => {

	const disposables = new DisposableStore();
	let service: AIProviderService;

	ensureNoDisposablesAreLeakedInTestSuite();

	setup(() => {
		service = disposables.add(new AIProviderService(new NullLogService()));
	});

	teardown(() => {
		disposables.clear();
	});

	test('registerProvider adds a provider that can be retrieved with getProvider', () => {
		const provider = makeMockProvider('anthropic');

		service.registerProvider('anthropic', provider);

		assert.strictEqual(service.getProvider('anthropic'), provider);
	});

	test('getProvider returns undefined for unregistered provider', () => {
		assert.strictEqual(service.getProvider('nonexistent'), undefined);
	});

	test('getProvider returns undefined for unregistered provider after other registrations', () => {
		service.registerProvider('anthropic', makeMockProvider('anthropic'));

		assert.strictEqual(service.getProvider('openai'), undefined);
	});

	test('getActiveProvider returns undefined when no active provider is set', () => {
		assert.strictEqual(service.getActiveProvider(), undefined);
	});

	test('getActiveProvider returns undefined even after registering a provider', () => {
		service.registerProvider('anthropic', makeMockProvider('anthropic'));

		assert.strictEqual(service.getActiveProvider(), undefined);
	});

	test('setActiveProvider then getActiveProvider returns the correct provider', () => {
		const provider = makeMockProvider('anthropic');
		service.registerProvider('anthropic', provider);

		service.setActiveProvider('anthropic');

		assert.strictEqual(service.getActiveProvider(), provider);
	});

	test('setActiveProvider fires onDidChangeProvider event with provider name', () => {
		const provider = makeMockProvider('anthropic');
		service.registerProvider('anthropic', provider);

		const fired: string[] = [];
		disposables.add(service.onDidChangeProvider(name => fired.push(name)));

		service.setActiveProvider('anthropic');

		assert.deepStrictEqual(fired, ['anthropic']);
	});

	test('setActiveProvider fires onDidChangeProvider each time it is called', () => {
		service.registerProvider('anthropic', makeMockProvider('anthropic'));
		service.registerProvider('openai', makeMockProvider('openai'));

		const fired: string[] = [];
		disposables.add(service.onDidChangeProvider(name => fired.push(name)));

		service.setActiveProvider('anthropic');
		service.setActiveProvider('openai');

		assert.deepStrictEqual(fired, ['anthropic', 'openai']);
	});

	test('listProviders returns all registered provider names', () => {
		service.registerProvider('anthropic', makeMockProvider('anthropic'));
		service.registerProvider('openai', makeMockProvider('openai'));
		service.registerProvider('local', makeMockProvider('local'));

		assert.deepStrictEqual(service.listProviders(), ['anthropic', 'openai', 'local']);
	});

	test('listProviders returns empty array when no providers registered', () => {
		assert.deepStrictEqual(service.listProviders(), []);
	});

	test('switching active provider updates getActiveProvider', () => {
		const anthropic = makeMockProvider('anthropic');
		const openai = makeMockProvider('openai');
		service.registerProvider('anthropic', anthropic);
		service.registerProvider('openai', openai);

		service.setActiveProvider('anthropic');
		assert.strictEqual(service.getActiveProvider(), anthropic);

		service.setActiveProvider('openai');
		assert.strictEqual(service.getActiveProvider(), openai);
	});
});
