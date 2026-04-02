/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import assert from 'assert';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../base/test/common/utils.js';
import type { IAIProvider, AICompletionRequest, AICompletionResponse, AIStreamChunk } from '../../common/aiProvider.js';
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

	const disposables = ensureNoDisposablesAreLeakedInTestSuite();
	let service: AIProviderService;

	setup(() => {
		service = disposables.add(new AIProviderService(new NullLogService()));
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

	test('has returns true for registered provider and false for unregistered', () => {
		service.registerProvider('anthropic', makeMockProvider('anthropic'));

		assert.strictEqual(service.has('anthropic'), true);
		assert.strictEqual(service.has('openai'), false);
	});

	test('getDefaultProviderName returns undefined when no default is set', () => {
		assert.strictEqual(service.getDefaultProviderName(), undefined);
	});

	test('setDefaultProviderName then getDefaultProviderName returns the name', () => {
		service.setDefaultProviderName('anthropic');

		assert.strictEqual(service.getDefaultProviderName(), 'anthropic');
	});

	test('registerProvider fires onDidChangeProviders with updated list', () => {
		const fired: string[][] = [];
		disposables.add(service.onDidChangeProviders(names => fired.push(names)));

		service.registerProvider('anthropic', makeMockProvider('anthropic'));

		assert.deepStrictEqual(fired, [['anthropic']]);
	});

	test('unregisterProvider fires onDidChangeProviders', () => {
		service.registerProvider('anthropic', makeMockProvider('anthropic'));
		service.registerProvider('openai', makeMockProvider('openai'));

		const fired: string[][] = [];
		disposables.add(service.onDidChangeProviders(names => fired.push(names)));

		service.unregisterProvider('anthropic');

		assert.deepStrictEqual(fired, [['openai']]);
		assert.strictEqual(service.has('anthropic'), false);
	});

	test('unregisterProvider clears default if it matches', () => {
		service.registerProvider('anthropic', makeMockProvider('anthropic'));
		service.setDefaultProviderName('anthropic');

		service.unregisterProvider('anthropic');

		assert.strictEqual(service.getDefaultProviderName(), undefined);
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

	test('registerProvider fires onDidChangeProviders each time', () => {
		const fired: string[][] = [];
		disposables.add(service.onDidChangeProviders(names => fired.push(names)));

		service.registerProvider('anthropic', makeMockProvider('anthropic'));
		service.registerProvider('openai', makeMockProvider('openai'));

		assert.deepStrictEqual(fired, [['anthropic'], ['anthropic', 'openai']]);
	});

	// --- Phase 3.5: New interface methods ---
	// These tests target the updated IAIProviderService interface.
	// They will pass once the service is updated to include:
	//   unregisterProvider, has, onDidChangeProviders, getDefaultProviderName

	suite('Phase 3.5 — updated interface', () => {

		test('unregisterProvider removes a previously registered provider', () => {
			service.registerProvider('anthropic', makeMockProvider('anthropic'));

			service.unregisterProvider('anthropic');

			assert.strictEqual(service.getProvider('anthropic'), undefined);
		});

		test('unregisterProvider removes provider from listProviders', () => {
			service.registerProvider('anthropic', makeMockProvider('anthropic'));
			service.registerProvider('openai', makeMockProvider('openai'));

			service.unregisterProvider('anthropic');

			assert.deepStrictEqual(service.listProviders(), ['openai']);
		});

		test('has returns true for registered provider', () => {
			service.registerProvider('anthropic', makeMockProvider('anthropic'));

			assert.strictEqual(service.has('anthropic'), true);
		});

		test('has returns false for unregistered provider', () => {
			assert.strictEqual(service.has('nonexistent'), false);
		});

		test('has returns false after unregisterProvider', () => {
			service.registerProvider('anthropic', makeMockProvider('anthropic'));
			service.unregisterProvider('anthropic');

			assert.strictEqual(service.has('anthropic'), false);
		});

		test('onDidChangeProviders fires on registerProvider', () => {
			const fired: number[] = [];
			disposables.add(service.onDidChangeProviders(() => fired.push(1)));

			service.registerProvider('anthropic', makeMockProvider('anthropic'));

			assert.strictEqual(fired.length, 1, 'onDidChangeProviders should fire once on register');
		});

		test('onDidChangeProviders fires on unregisterProvider', () => {
			service.registerProvider('anthropic', makeMockProvider('anthropic'));

			const fired: number[] = [];
			disposables.add(service.onDidChangeProviders(() => fired.push(1)));

			service.unregisterProvider('anthropic');

			assert.strictEqual(fired.length, 1, 'onDidChangeProviders should fire once on unregister');
		});

		test('onDidChangeProviders fires for each register and unregister call', () => {
			const fired: number[] = [];
			disposables.add(service.onDidChangeProviders(() => fired.push(1)));

			service.registerProvider('anthropic', makeMockProvider('anthropic'));
			service.registerProvider('openai', makeMockProvider('openai'));
			service.unregisterProvider('anthropic');

			assert.strictEqual(fired.length, 3, 'onDidChangeProviders should fire 3 times');
		});

		test('getDefaultProviderName returns the value set via setDefaultProviderName', () => {
			service.setDefaultProviderName('anthropic');

			const defaultName = service.getDefaultProviderName();

			assert.strictEqual(defaultName, 'anthropic');
		});

		test('listProviders returns all registered provider names after multiple operations', () => {
			service.registerProvider('anthropic', makeMockProvider('anthropic'));
			service.registerProvider('openai', makeMockProvider('openai'));
			service.registerProvider('local', makeMockProvider('local'));
			service.unregisterProvider('openai');

			assert.deepStrictEqual(service.listProviders(), ['anthropic', 'local']);
		});

		test('unregisterProvider on nonexistent name does not throw', () => {
			assert.doesNotThrow(() => {
				service.unregisterProvider('nonexistent');
			});
		});

		test('re-register after unregister returns new instance', () => {
			const first = makeMockProvider('anthropic');
			const second = makeMockProvider('anthropic');

			service.registerProvider('anthropic', first);
			service.unregisterProvider('anthropic');
			service.registerProvider('anthropic', second);

			assert.strictEqual(service.getProvider('anthropic'), second);
			assert.notStrictEqual(service.getProvider('anthropic'), first);
		});

		test('unregister non-default provider preserves default', () => {
			service.registerProvider('anthropic', makeMockProvider('anthropic'));
			service.registerProvider('openai', makeMockProvider('openai'));
			service.setDefaultProviderName('anthropic');

			service.unregisterProvider('openai');

			assert.strictEqual(service.getDefaultProviderName(), 'anthropic');
		});

		test('multiple listeners all receive onDidChangeProviders', () => {
			const firedA: string[][] = [];
			const firedB: string[][] = [];
			disposables.add(service.onDidChangeProviders(names => firedA.push(names)));
			disposables.add(service.onDidChangeProviders(names => firedB.push(names)));

			service.registerProvider('anthropic', makeMockProvider('anthropic'));

			assert.deepStrictEqual(firedA, [['anthropic']]);
			assert.deepStrictEqual(firedB, [['anthropic']]);
		});
	});
});
