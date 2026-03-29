/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import assert from 'assert';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../base/test/common/utils.js';
import { MistralProvider } from '../../node/mistralProvider.js';
import type { AICompletionRequest } from '../../common/aiProvider.js';

function makeClient(deltas: string[]): unknown {
	return {
		chat: {
			stream: async () => ({
				[Symbol.asyncIterator]: async function* () {
					for (const content of deltas) {
						yield { data: { choices: [{ delta: { content } }] } };
					}
				}
			}),
			complete: async () => ({
				choices: [{ message: { content: 'done' } }],
				model: 'mistral-large-latest',
				usage: { promptTokens: 5, completionTokens: 3 },
			}),
		},
		models: {
			list: async () => ({ data: [] }),
		},
	};
}

function makeRequest(overrides?: Partial<AICompletionRequest>): AICompletionRequest {
	return {
		messages: [{ role: 'user', content: 'hello' }],
		model: 'mistral-large-latest',
		...overrides,
	};
}

suite('MistralProvider', () => {

	ensureNoDisposablesAreLeakedInTestSuite();

	test('stream yields content deltas', async () => {
		const client = makeClient(['Hello', ' world']);
		const provider = new MistralProvider(client as ConstructorParameters<typeof MistralProvider>[0]);

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
		const provider = new MistralProvider(client as ConstructorParameters<typeof MistralProvider>[0]);

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
		const provider = new MistralProvider(client as ConstructorParameters<typeof MistralProvider>[0]);

		const response = await provider.complete(makeRequest());

		assert.strictEqual(response.content, 'done');
		assert.strictEqual(response.model, 'mistral-large-latest');
		assert.strictEqual(response.inputTokens, 5);
		assert.strictEqual(response.outputTokens, 3);
	});

	test('availableModels includes mistral-large-latest and codestral-latest', () => {
		const client = makeClient([]);
		const provider = new MistralProvider(client as ConstructorParameters<typeof MistralProvider>[0]);

		assert.ok(provider.availableModels.includes('mistral-large-latest'));
		assert.ok(provider.availableModels.includes('codestral-latest'));
	});
});
