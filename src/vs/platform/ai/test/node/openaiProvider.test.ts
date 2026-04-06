import assert from 'assert';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../base/test/common/utils.js';
import { OpenAIProvider } from '../../node/openaiProvider.js';
import type { AICompletionRequest, AIStreamChunk, AIToolDefinition } from '../../common/aiProvider.js';

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

	test('stream done chunk carries usage when stream_options include_usage', async () => {
		const mockClient: unknown = {
			chat: {
				completions: {
					create: () => ({
						[Symbol.asyncIterator]: async function* () {
							yield { choices: [{ delta: { content: 'Hi' }, finish_reason: null }], usage: null };
							yield { choices: [{ delta: {}, finish_reason: 'stop' }], usage: null };
							yield { choices: [], usage: { prompt_tokens: 8, completion_tokens: 4 } };
						}
					})
				}
			},
			models: { list: async () => ({ data: [] }) },
		};
		const provider = new OpenAIProvider(mockClient as ConstructorParameters<typeof OpenAIProvider>[0]);

		const chunks: AIStreamChunk[] = [];
		for await (const chunk of provider.stream(makeRequest())) {
			chunks.push(chunk);
		}

		const done = chunks[chunks.length - 1];
		assert.strictEqual(done.done, true);
		assert.ok(done.usage);
		assert.strictEqual(done.usage!.inputTokens, 8);
		assert.strictEqual(done.usage!.outputTokens, 4);
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

suite('OpenAIProvider tool calling', () => {

	ensureNoDisposablesAreLeakedInTestSuite();

	test('stream() converts AIToolDefinition to OpenAI tools format', async () => {
		const toolDef: AIToolDefinition = {
			name: 'read_file',
			description: 'Read a file',
			inputSchema: {
				type: 'object' as const,
				properties: { path: { type: 'string' } }
			}
		};

		let capturedRequest: Record<string, unknown>;
		const mockClient: unknown = {
			chat: {
				completions: {
					create: (req: Record<string, unknown>) => {
						capturedRequest = req;
						return {
							[Symbol.asyncIterator]: async function* () {
								yield {
									choices: [{
										delta: {
											tool_calls: [{
												index: 0,
												id: 'call_abc',
												function: { name: 'read_file', arguments: '{"path":"/tmp/t.txt"}' }
											}]
										},
										finish_reason: null
									}]
								};
								yield {
									choices: [{
										delta: {},
										finish_reason: 'tool_calls'
									}]
								};
							}
						};
					}
				}
			},
			models: {
				list: async () => ({ data: [] }),
			},
		};

		const provider = new OpenAIProvider(mockClient as ConstructorParameters<typeof OpenAIProvider>[0]);

		const chunks: AIStreamChunk[] = [];
		for await (const chunk of provider.stream({
			messages: [{ role: 'user', content: 'Read /tmp/t.txt' }],
			model: 'gpt-4o',
			tools: [toolDef]
		})) {
			chunks.push(chunk);
		}

		const tools = capturedRequest!.tools as Array<{ type: string; function: { name: string } }>;
		assert.ok(tools);
		assert.strictEqual(tools[0].type, 'function');
		assert.strictEqual(tools[0].function.name, 'read_file');

		const toolChunk = chunks.find(c => c.toolUse);
		assert.ok(toolChunk);
		assert.strictEqual(toolChunk!.toolUse!.name, 'read_file');
	});

	test('stream() handles tool_result messages as tool role', async () => {
		let capturedRequest: Record<string, unknown>;
		const mockClient: unknown = {
			chat: {
				completions: {
					create: (req: Record<string, unknown>) => {
						capturedRequest = req;
						return {
							[Symbol.asyncIterator]: async function* () {
								yield { choices: [{ delta: { content: 'OK' }, finish_reason: 'stop' }] };
							}
						};
					}
				}
			},
			models: {
				list: async () => ({ data: [] }),
			},
		};

		const provider = new OpenAIProvider(mockClient as ConstructorParameters<typeof OpenAIProvider>[0]);

		for await (const _ of provider.stream({
			messages: [
				{ role: 'user', content: 'Read file' },
				{ role: 'assistant', content: '' },
				{ role: 'tool_result', content: 'contents', toolCallId: 'call_abc' }
			],
			model: 'gpt-4o'
		})) { /* drain */ }

		const messages = capturedRequest!.messages as Array<{ role: string; tool_call_id?: string }>;
		const toolMsg = messages.find((m) => m.role === 'tool');
		assert.ok(toolMsg);
		assert.strictEqual(toolMsg.tool_call_id, 'call_abc');
	});

	test('stream() throws on tool_result message without toolCallId', async () => {
		const mockClient: unknown = {
			chat: {
				completions: {
					create: () => ({
						[Symbol.asyncIterator]: async function* () {
							yield { choices: [{ delta: { content: 'x' }, finish_reason: 'stop' }] };
						}
					})
				}
			},
			models: {
				list: async () => ({ data: [] }),
			},
		};

		const provider = new OpenAIProvider(mockClient as ConstructorParameters<typeof OpenAIProvider>[0]);

		await assert.rejects(
			async () => {
				for await (const _ of provider.stream({
					messages: [{ role: 'tool_result', content: 'data' }],
					model: 'gpt-4o'
				})) { /* drain */ }
			},
			/toolCallId/
		);
	});

	test('stream() text-only response has no toolUse in chunks', async () => {
		const mockClient: unknown = {
			chat: {
				completions: {
					create: () => ({
						[Symbol.asyncIterator]: async function* () {
							yield { choices: [{ delta: { content: 'Hello' }, finish_reason: null }] };
							yield { choices: [{ delta: {}, finish_reason: 'stop' }] };
						}
					})
				}
			},
			models: { list: async () => ({ data: [] }) },
		};

		const provider = new OpenAIProvider(mockClient as ConstructorParameters<typeof OpenAIProvider>[0]);
		const chunks: AIStreamChunk[] = [];
		for await (const chunk of provider.stream({ messages: [{ role: 'user', content: 'hi' }], model: 'gpt-4o' })) {
			chunks.push(chunk);
		}

		assert.ok(chunks.length > 0);
		assert.ok(chunks.every(c => c.toolUse === undefined));
	});

	test('stream() toolUse chunk includes id and parsed input', async () => {
		const mockClient: unknown = {
			chat: {
				completions: {
					create: () => ({
						[Symbol.asyncIterator]: async function* () {
							yield {
								choices: [{
									delta: {
										tool_calls: [{
											index: 0,
											id: 'call_xyz',
											function: { name: 'write_file', arguments: '{"path":"/tmp/out.txt","content":"hello"}' }
										}]
									},
									finish_reason: null
								}]
							};
							yield { choices: [{ delta: {}, finish_reason: 'tool_calls' }] };
						}
					})
				}
			},
			models: { list: async () => ({ data: [] }) },
		};

		const provider = new OpenAIProvider(mockClient as ConstructorParameters<typeof OpenAIProvider>[0]);
		const chunks: AIStreamChunk[] = [];
		for await (const chunk of provider.stream({
			messages: [{ role: 'user', content: 'write it' }],
			model: 'gpt-4o',
			tools: [{ name: 'write_file', description: 'Write a file', inputSchema: {} }]
		})) {
			chunks.push(chunk);
		}

		const toolChunk = chunks.find(c => c.toolUse);
		assert.ok(toolChunk);
		assert.strictEqual(toolChunk!.toolUse!.id, 'call_xyz');
		assert.strictEqual(toolChunk!.toolUse!.name, 'write_file');
		assert.deepStrictEqual(toolChunk!.toolUse!.input, { path: '/tmp/out.txt', content: 'hello' });
	});
});
