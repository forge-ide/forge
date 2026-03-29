/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import assert from 'assert';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../base/test/common/utils.js';
import { AnthropicProvider } from '../../node/anthropicProvider.js';
import type { AICompletionRequest } from '../../common/aiProvider.js';

function makeClient(chunks: string[]): unknown {
	return {
		messages: {
			stream: () => ({
				[Symbol.asyncIterator]: async function* () {
					for (const text of chunks) {
						yield { type: 'content_block_delta', delta: { type: 'text_delta', text } };
					}
				}
			}),
			create: async () => ({
				content: [{ text: 'done', type: 'text' }],
				model: 'claude-sonnet-4-6',
				usage: { input_tokens: 5, output_tokens: 3 },
			}),
		},
	};
}

function makeRequest(overrides?: Partial<AICompletionRequest>): AICompletionRequest {
	return {
		messages: [{ role: 'user', content: 'hello' }],
		model: 'claude-sonnet-4-6',
		...overrides,
	};
}

suite('AnthropicProvider', () => {

	ensureNoDisposablesAreLeakedInTestSuite();

	test('stream yields one chunk per token', async () => {
		const client = makeClient(['Hello', ' world']);
		const provider = new AnthropicProvider(client as ConstructorParameters<typeof AnthropicProvider>[0]);

		const chunks = [];
		for await (const chunk of provider.stream(makeRequest())) {
			chunks.push(chunk);
		}

		// Two content chunks plus one final done chunk
		assert.strictEqual(chunks.length, 3);
		assert.strictEqual(chunks[0].delta, 'Hello');
		assert.strictEqual(chunks[0].done, false);
		assert.strictEqual(chunks[1].delta, ' world');
		assert.strictEqual(chunks[1].done, false);
	});

	test('stream yields a final done chunk', async () => {
		const client = makeClient(['Hi']);
		const provider = new AnthropicProvider(client as ConstructorParameters<typeof AnthropicProvider>[0]);

		const chunks = [];
		for await (const chunk of provider.stream(makeRequest())) {
			chunks.push(chunk);
		}

		const last = chunks[chunks.length - 1];
		assert.strictEqual(last.delta, '');
		assert.strictEqual(last.done, true);
	});

	test('complete returns AICompletionResponse with correct fields', async () => {
		const client = makeClient([]);
		const provider = new AnthropicProvider(client as ConstructorParameters<typeof AnthropicProvider>[0]);

		const response = await provider.complete(makeRequest());

		assert.strictEqual(response.content, 'done');
		assert.strictEqual(response.model, 'claude-sonnet-4-6');
		assert.strictEqual(response.inputTokens, 5);
		assert.strictEqual(response.outputTokens, 3);
	});

	test('availableModels includes claude-sonnet-4-6', () => {
		const client = makeClient([]);
		const provider = new AnthropicProvider(client as ConstructorParameters<typeof AnthropicProvider>[0]);

		assert.ok(provider.availableModels.includes('claude-sonnet-4-6'));
	});
});
