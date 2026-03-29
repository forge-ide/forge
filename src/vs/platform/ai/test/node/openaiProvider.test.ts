/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import assert from 'assert';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../base/test/common/utils.js';
import { OpenAIProvider } from '../../node/openaiProvider.js';
import type { AICompletionRequest } from '../../common/aiProvider.js';

function makeClient(deltas: string[]): unknown {
	return {
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
						model: 'gpt-4o',
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

function makeRequest(overrides?: Partial<AICompletionRequest>): AICompletionRequest {
	return {
		messages: [{ role: 'user', content: 'hello' }],
		model: 'gpt-4o',
		...overrides,
	};
}

suite('OpenAIProvider', () => {

	ensureNoDisposablesAreLeakedInTestSuite();

	test('stream yields content deltas', async () => {
		const client = makeClient(['Hello', ' world']);
		const provider = new OpenAIProvider(client as ConstructorParameters<typeof OpenAIProvider>[0]);

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
		const client = makeClient(['Hi']);
		const provider = new OpenAIProvider(client as ConstructorParameters<typeof OpenAIProvider>[0]);

		const chunks = [];
		for await (const chunk of provider.stream(makeRequest())) {
			chunks.push(chunk);
		}

		const last = chunks[chunks.length - 1];
		assert.strictEqual(last.delta, '');
		assert.strictEqual(last.done, true);
	});

	test('complete returns AICompletionResponse', async () => {
		const client = makeClient([]);
		const provider = new OpenAIProvider(client as ConstructorParameters<typeof OpenAIProvider>[0]);

		const response = await provider.complete(makeRequest());

		assert.strictEqual(response.content, 'done');
		assert.strictEqual(response.model, 'gpt-4o');
		assert.strictEqual(response.inputTokens, 5);
		assert.strictEqual(response.outputTokens, 3);
	});

	test('availableModels includes gpt-4o', () => {
		const client = makeClient([]);
		const provider = new OpenAIProvider(client as ConstructorParameters<typeof OpenAIProvider>[0]);

		assert.ok(provider.availableModels.includes('gpt-4o'));
	});
});
