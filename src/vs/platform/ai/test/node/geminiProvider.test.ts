/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import assert from 'assert';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../base/test/common/utils.js';
import { GeminiProvider } from '../../node/geminiProvider.js';
import type { AICompletionRequest } from '../../common/aiProvider.js';

function makeClient(texts: string[]): unknown {
	return {
		getGenerativeModel: () => ({
			generateContentStream: async () => ({
				stream: (async function* () {
					for (const text of texts) {
						yield { text: () => text };
					}
				})(),
			}),
			generateContent: async () => ({
				response: {
					text: () => 'done',
					usageMetadata: { promptTokenCount: 5, candidatesTokenCount: 3 },
				},
			}),
		}),
	};
}

function makeRequest(overrides?: Partial<AICompletionRequest>): AICompletionRequest {
	return {
		messages: [{ role: 'user', content: 'hello' }],
		model: 'gemini-2.0-flash',
		...overrides,
	};
}

suite('GeminiProvider', () => {

	ensureNoDisposablesAreLeakedInTestSuite();

	test('stream yields text deltas', async () => {
		const client = makeClient(['Hello', ' world']);
		const provider = new GeminiProvider(
			client as ConstructorParameters<typeof GeminiProvider>[0],
			'gemini-2.0-flash',
		);

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
		const provider = new GeminiProvider(
			client as ConstructorParameters<typeof GeminiProvider>[0],
			'gemini-2.0-flash',
		);

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
		const provider = new GeminiProvider(
			client as ConstructorParameters<typeof GeminiProvider>[0],
			'gemini-2.0-flash',
		);

		const response = await provider.complete(makeRequest());

		assert.strictEqual(response.content, 'done');
		assert.strictEqual(response.model, 'gemini-2.0-flash');
		assert.strictEqual(response.inputTokens, 5);
		assert.strictEqual(response.outputTokens, 3);
	});

	test('availableModels includes gemini-2.0-flash', () => {
		const client = makeClient([]);
		const provider = new GeminiProvider(
			client as ConstructorParameters<typeof GeminiProvider>[0],
			'gemini-2.0-flash',
		);

		assert.ok(provider.availableModels.includes('gemini-2.0-flash'));
	});
});
