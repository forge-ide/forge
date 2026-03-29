/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import assert from 'assert';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../base/test/common/utils.js';
import { LocalProvider } from '../../node/localProvider.js';
import type { AICompletionRequest } from '../../common/aiProvider.js';

function makeOpenAIClient(deltas: string[]): unknown {
	return {
		baseURL: 'http://localhost:11434/v1',
		chat: {
			completions: {
				create: async (opts: Record<string, unknown>) => {
					if (opts.stream) {
						return {
							[Symbol.asyncIterator]: async function* () {
								for (const content of deltas) {
									yield { choices: [{ delta: { content } }] };
								}
							}
						};
					}
					return {
						choices: [{ message: { content: 'done' } }],
						model: 'llama3',
						usage: { prompt_tokens: 5, completion_tokens: 3 },
					};
				},
			},
		},
		models: {
			list: async () => ({ data: [] }),
		},
	};
}

function patchClient(provider: LocalProvider, mockClient: unknown): void {
	(provider as unknown as { client: unknown }).client = mockClient;
}

function makeRequest(overrides?: Partial<AICompletionRequest>): AICompletionRequest {
	return {
		messages: [{ role: 'user', content: 'hello' }],
		model: 'llama3',
		...overrides,
	};
}

suite('LocalProvider', () => {

	ensureNoDisposablesAreLeakedInTestSuite();

	test('name is "local"', () => {
		const provider = new LocalProvider({ baseURL: 'http://localhost:11434/v1' });

		assert.strictEqual(provider.name, 'local');
	});

	test('availableModels returns empty array', () => {
		const provider = new LocalProvider({ baseURL: 'http://localhost:11434/v1' });

		assert.deepStrictEqual(provider.availableModels, []);
	});

	test('stream yields content deltas', async () => {
		const provider = new LocalProvider({ baseURL: 'http://localhost:11434/v1' });
		patchClient(provider, makeOpenAIClient(['Hello', ' world']));

		const chunks = [];
		for await (const chunk of provider.stream(makeRequest())) {
			chunks.push(chunk);
		}

		assert.strictEqual(chunks.length, 3);
		assert.strictEqual(chunks[0].delta, 'Hello');
		assert.strictEqual(chunks[0].done, false);
		assert.strictEqual(chunks[1].delta, ' world');
		assert.strictEqual(chunks[1].done, false);
	});

	test('stream yields final done chunk', async () => {
		const provider = new LocalProvider({ baseURL: 'http://localhost:11434/v1' });
		patchClient(provider, makeOpenAIClient(['Hi']));

		const chunks = [];
		for await (const chunk of provider.stream(makeRequest())) {
			chunks.push(chunk);
		}

		const last = chunks[chunks.length - 1];
		assert.strictEqual(last.delta, '');
		assert.strictEqual(last.done, true);
	});

	test('complete returns AICompletionResponse', async () => {
		const provider = new LocalProvider({ baseURL: 'http://localhost:11434/v1' });
		patchClient(provider, makeOpenAIClient([]));

		const response = await provider.complete(makeRequest());

		assert.strictEqual(response.content, 'done');
		assert.strictEqual(response.model, 'llama3');
		assert.strictEqual(response.inputTokens, 5);
		assert.strictEqual(response.outputTokens, 3);
	});
});
