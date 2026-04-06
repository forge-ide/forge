import assert from 'assert';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../base/test/common/utils.js';
import { AnthropicProvider } from '../../node/anthropicProvider.js';
import type { AICompletionRequest, AIStreamChunk, AIToolDefinition } from '../../common/aiProvider.js';

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

	test('stream done chunk carries usage from message_start and message_delta', async () => {
		const mockClient: unknown = {
			messages: {
				stream: () => ({
					[Symbol.asyncIterator]: async function* () {
						yield { type: 'message_start', message: { usage: { input_tokens: 12, output_tokens: 0 } } };
						yield { type: 'content_block_delta', delta: { type: 'text_delta', text: 'Hello' } };
						yield { type: 'message_delta', usage: { output_tokens: 7 } };
					}
				}),
				create: async () => ({ content: [], model: 'claude-sonnet-4-6', usage: { input_tokens: 0, output_tokens: 0 } }),
			},
		};
		const provider = new AnthropicProvider(mockClient as ConstructorParameters<typeof AnthropicProvider>[0]);

		const chunks: AIStreamChunk[] = [];
		for await (const chunk of provider.stream(makeRequest())) {
			chunks.push(chunk);
		}

		const done = chunks[chunks.length - 1];
		assert.strictEqual(done.done, true);
		assert.ok(done.usage);
		assert.strictEqual(done.usage!.inputTokens, 12);
		assert.strictEqual(done.usage!.outputTokens, 7);
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

suite('AnthropicProvider tool calling', () => {

	ensureNoDisposablesAreLeakedInTestSuite();

	test('stream() converts AIToolDefinition to Anthropic tool format', async () => {
		const toolDef: AIToolDefinition = {
			name: 'read_file',
			description: 'Read a file',
			inputSchema: {
				type: 'object' as const,
				properties: { path: { type: 'string' } },
				required: ['path']
			}
		};

		let capturedRequest: unknown;
		const mockClient = {
			messages: {
				stream: (req: unknown) => {
					capturedRequest = req;
					return {
						[Symbol.asyncIterator]: async function* () {
							yield { type: 'content_block_start', index: 0, content_block: { type: 'tool_use', id: 'call_1', name: 'read_file', input: {} } };
							yield { type: 'content_block_delta', index: 0, delta: { type: 'input_json_delta', partial_json: '{"path":"/tmp/t.txt"}' } };
							yield { type: 'content_block_stop', index: 0 };
							yield { type: 'message_stop' };
						}
					};
				}
			}
		};

		const provider = new AnthropicProvider(mockClient as unknown as ConstructorParameters<typeof AnthropicProvider>[0]);

		const chunks: AIStreamChunk[] = [];
		for await (const chunk of provider.stream({
			messages: [{ role: 'user', content: 'Read /tmp/t.txt' }],
			model: 'claude-sonnet-4-6',
			tools: [toolDef]
		})) {
			chunks.push(chunk);
		}

		const req = capturedRequest as Record<string, unknown>;
		assert.ok(req['tools']);
		assert.strictEqual((req['tools'] as Array<Record<string, unknown>>)[0]['name'], 'read_file');

		const toolChunk = chunks.find(c => c.toolUse);
		assert.ok(toolChunk);
		assert.strictEqual(toolChunk!.toolUse!.name, 'read_file');
		assert.strictEqual(toolChunk!.toolUse!.id, 'call_1');
	});

	test('stream() works without tools (backward compatible)', async () => {
		const mockClient = {
			messages: {
				stream: () => ({
					[Symbol.asyncIterator]: async function* () {
						yield { type: 'content_block_delta', index: 0, delta: { type: 'text_delta', text: 'Hello' } };
						yield { type: 'message_stop' };
					}
				})
			}
		};

		const provider = new AnthropicProvider(mockClient as unknown as ConstructorParameters<typeof AnthropicProvider>[0]);

		const chunks: AIStreamChunk[] = [];
		for await (const chunk of provider.stream({
			messages: [{ role: 'user', content: 'Hi' }],
			model: 'claude-sonnet-4-6'
		})) {
			chunks.push(chunk);
		}

		assert.ok(chunks.length > 0);
		assert.strictEqual(chunks[0].delta, 'Hello');
		assert.strictEqual(chunks[0].toolUse, undefined);
	});

	test('stream() handles tool_result messages', async () => {
		let capturedRequest: unknown;
		const mockClient = {
			messages: {
				stream: (req: unknown) => {
					capturedRequest = req;
					return {
						[Symbol.asyncIterator]: async function* () {
							yield { type: 'content_block_delta', index: 0, delta: { type: 'text_delta', text: 'Done' } };
							yield { type: 'message_stop' };
						}
					};
				}
			}
		};

		const provider = new AnthropicProvider(mockClient as unknown as ConstructorParameters<typeof AnthropicProvider>[0]);

		const chunks: AIStreamChunk[] = [];
		for await (const chunk of provider.stream({
			messages: [
				{ role: 'user', content: 'Read my file' },
				{ role: 'assistant', content: '' },
				{ role: 'tool_result', content: 'file contents', toolCallId: 'call_1' }
			],
			model: 'claude-sonnet-4-6'
		})) {
			chunks.push(chunk);
		}

		const req = capturedRequest as Record<string, unknown>;
		const toolResultMsg = (req['messages'] as Array<Record<string, unknown>>).find(
			(m) => m['role'] === 'user' && Array.isArray(m['content']) && (m['content'] as Array<Record<string, unknown>>)[0]?.['type'] === 'tool_result'
		);
		assert.ok(toolResultMsg, 'tool_result should be converted to user message with tool_result content block');

		const block = (toolResultMsg['content'] as Array<Record<string, unknown>>)[0];
		assert.strictEqual(block['tool_use_id'], 'call_1');
		assert.strictEqual(block['content'], 'file contents');
	});

	test('stream() toolUse chunk includes parsed input', async () => {
		const mockClient = {
			messages: {
				stream: () => ({
					[Symbol.asyncIterator]: async function* () {
						yield { type: 'content_block_start', index: 0, content_block: { type: 'tool_use', id: 'call_x', name: 'read_file', input: {} } };
						yield { type: 'content_block_delta', index: 0, delta: { type: 'input_json_delta', partial_json: '{"path":"/etc/hosts"}' } };
						yield { type: 'content_block_stop', index: 0 };
						yield { type: 'message_stop' };
					}
				})
			}
		};

		const provider = new AnthropicProvider(mockClient as unknown as ConstructorParameters<typeof AnthropicProvider>[0]);
		const chunks: AIStreamChunk[] = [];
		for await (const chunk of provider.stream({ messages: [{ role: 'user', content: 'go' }], model: 'claude-sonnet-4-6', tools: [{ name: 'read_file', description: 'Read', inputSchema: {} }] })) {
			chunks.push(chunk);
		}

		const toolChunk = chunks.find(c => c.toolUse);
		assert.ok(toolChunk);
		assert.strictEqual(toolChunk!.toolUse!.id, 'call_x');
		assert.strictEqual(toolChunk!.toolUse!.name, 'read_file');
		assert.deepStrictEqual(toolChunk!.toolUse!.input, { path: '/etc/hosts' });
	});

	test('stream() omits tools from API call when none in request', async () => {
		let capturedRequest: unknown;
		const mockClient = {
			messages: {
				stream: (req: unknown) => {
					capturedRequest = req;
					return {
						[Symbol.asyncIterator]: async function* () {
							yield { type: 'content_block_delta', index: 0, delta: { type: 'text_delta', text: 'Hi' } };
							yield { type: 'message_stop' };
						}
					};
				}
			}
		};

		const provider = new AnthropicProvider(mockClient as unknown as ConstructorParameters<typeof AnthropicProvider>[0]);
		for await (const _ of provider.stream({ messages: [{ role: 'user', content: 'hello' }], model: 'claude-sonnet-4-6' })) { /* drain */ }

		const req = capturedRequest as Record<string, unknown>;
		assert.strictEqual(req['tools'], undefined);
	});
});
