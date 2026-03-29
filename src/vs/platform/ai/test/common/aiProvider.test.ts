/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import assert from 'assert';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../base/test/common/utils.js';
import type {
	IAIProvider,
	AIMessage,
	AICompletionRequest,
	AIStreamChunk,
	AICompletionResponse,
	AIValidationResult,
} from '../../common/aiProvider.js';

/**
 * Concrete implementation of IAIProvider used to verify the interface contract.
 * TypeScript will emit a compile error if any required member is missing or
 * has the wrong type — so a successful compile is itself the assertion.
 */
class MockAIProvider implements IAIProvider {
	readonly name: string = 'mock';
	readonly availableModels: string[] = ['mock-model'];

	complete(request: AICompletionRequest): Promise<AICompletionResponse> {
		return Promise.resolve({
			content: 'hello',
			model: request.model,
			inputTokens: 1,
			outputTokens: 1,
		});
	}

	async *stream(_request: AICompletionRequest): AsyncIterable<AIStreamChunk> {
		yield { delta: 'chunk', done: false };
		yield { delta: '', done: true };
	}

	validateCredentials(): Promise<AIValidationResult> {
		return Promise.resolve({ valid: true });
	}
}

suite('AIProvider interface contract', () => {

	ensureNoDisposablesAreLeakedInTestSuite();

	test('MockAIProvider satisfies the IAIProvider shape', () => {
		const provider: IAIProvider = new MockAIProvider();
		assert.strictEqual(typeof provider.name, 'string');
		assert.ok(Array.isArray(provider.availableModels));
		assert.strictEqual(typeof provider.complete, 'function');
		assert.strictEqual(typeof provider.stream, 'function');
		assert.strictEqual(typeof provider.validateCredentials, 'function');
	});

	test('AIMessage accepts role "user"', () => {
		const msg: AIMessage = { role: 'user', content: 'hello' };
		assert.strictEqual(msg.role, 'user');
	});

	test('AIMessage accepts role "assistant"', () => {
		const msg: AIMessage = { role: 'assistant', content: 'hello back' };
		assert.strictEqual(msg.role, 'assistant');
	});

	test('AIMessage accepts role "system"', () => {
		const msg: AIMessage = { role: 'system', content: 'you are helpful' };
		assert.strictEqual(msg.role, 'system');
	});

	test('AIStreamChunk with done:true has expected shape', () => {
		const chunk: AIStreamChunk = { delta: '', done: true };
		assert.strictEqual(chunk.done, true);
		assert.strictEqual(chunk.delta, '');
	});

	test('AIStreamChunk with done:false carries a delta string', () => {
		const chunk: AIStreamChunk = { delta: 'partial text', done: false };
		assert.strictEqual(chunk.done, false);
		assert.strictEqual(chunk.delta, 'partial text');
	});

	test('AIValidationResult with valid:true has no error', () => {
		const result: AIValidationResult = { valid: true };
		assert.strictEqual(result.valid, true);
		assert.strictEqual(result.error, undefined);
	});

	test('AIValidationResult with valid:false carries an error string', () => {
		const result: AIValidationResult = { valid: false, error: 'invalid api key' };
		assert.strictEqual(result.valid, false);
		assert.strictEqual(result.error, 'invalid api key');
	});

	test('AICompletionRequest requires messages and model', () => {
		const req: AICompletionRequest = {
			messages: [{ role: 'user', content: 'hi' }],
			model: 'claude-3',
		};
		assert.strictEqual(req.messages.length, 1);
		assert.strictEqual(req.model, 'claude-3');
		assert.strictEqual(req.maxTokens, undefined);
		assert.strictEqual(req.systemPrompt, undefined);
	});

	test('AICompletionRequest accepts optional maxTokens and systemPrompt', () => {
		const req: AICompletionRequest = {
			messages: [],
			model: 'claude-3',
			maxTokens: 1024,
			systemPrompt: 'be concise',
		};
		assert.strictEqual(req.maxTokens, 1024);
		assert.strictEqual(req.systemPrompt, 'be concise');
	});

	test('complete returns AICompletionResponse with expected fields', async () => {
		const provider = new MockAIProvider();
		const response = await provider.complete({ messages: [], model: 'mock-model' });

		assert.strictEqual(typeof response.content, 'string');
		assert.strictEqual(typeof response.model, 'string');
		assert.strictEqual(typeof response.inputTokens, 'number');
		assert.strictEqual(typeof response.outputTokens, 'number');
	});

	test('stream yields AIStreamChunk objects ending with done:true', async () => {
		const provider = new MockAIProvider();
		const chunks: AIStreamChunk[] = [];

		for await (const chunk of provider.stream({ messages: [], model: 'mock-model' })) {
			chunks.push(chunk);
		}

		assert.ok(chunks.length > 0);
		assert.strictEqual(chunks[chunks.length - 1].done, true);
	});
});
