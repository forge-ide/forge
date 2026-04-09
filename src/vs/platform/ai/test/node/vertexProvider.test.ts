import assert from 'assert';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../base/test/common/utils.js';
import { VertexProvider, type IGeminiModels, type IAnthropicVertexClient } from '../../node/vertexProvider.js';
import type { AICompletionRequest } from '../../common/aiProvider.js';

// --- Mock factories ---

type GeminiChunk = {
	text?: string;
	functionCall?: { name: string; args: Record<string, unknown> };
	usage?: { promptTokenCount: number; candidatesTokenCount: number };
};

function makeGeminiModels(streamChunks: GeminiChunk[] = [], completeText = 'done') {
	return {
		generateContentStream: async (_params: unknown) => {
			async function* gen() {
				for (const c of streamChunks) {
					const parts: unknown[] = [];
					if (c.text) { parts.push({ text: c.text }); }
					if (c.functionCall) { parts.push({ functionCall: c.functionCall }); }
					yield {
						candidates: parts.length ? [{ content: { parts } }] : [],
						usageMetadata: c.usage,
					};
				}
			}
			return gen();
		},
		generateContent: async (_params: unknown) => ({
			candidates: [{ content: { parts: [{ text: completeText }] } }],
			usageMetadata: { promptTokenCount: 5, candidatesTokenCount: 3 },
		}),
	};
}

function makeAnthropicClient(streamEvents: unknown[] = [], completeText = 'response') {
	return {
		messages: {
			stream: (_params: unknown) => {
				async function* gen() {
					for (const e of streamEvents) { yield e; }
				}
				return gen();
			},
			create: async (_params: unknown) => ({
				content: [{ type: 'text', text: completeText }],
				usage: { input_tokens: 5, output_tokens: 3 },
				model: 'claude-sonnet-4-5@20251001',
			}),
		},
	};
}

function makeRequest(overrides?: Partial<AICompletionRequest>): AICompletionRequest {
	return {
		messages: [{ role: 'user', content: 'hello' }],
		model: 'gemini-2.0-flash-001',
		...overrides,
	};
}

suite('VertexProvider', () => {

	ensureNoDisposablesAreLeakedInTestSuite();

	// --- Gemini stream ---

	test('stream() yields text deltas for gemini-* model', async () => {
		const gemini = makeGeminiModels([{ text: 'Hello' }, { text: ' world' }]);
		const provider = new VertexProvider(gemini as IGeminiModels, makeAnthropicClient() as IAnthropicVertexClient, ['gemini-2.0-flash-001']);

		const chunks = [];
		for await (const chunk of provider.stream(makeRequest({ model: 'gemini-2.0-flash-001' }))) {
			chunks.push(chunk);
		}

		const textChunks = chunks.filter(c => !c.done);
		assert.deepStrictEqual(textChunks.map(c => c.delta), ['Hello', ' world']);
	});

	test('stream() emits final done chunk with usage for gemini model', async () => {
		const gemini = makeGeminiModels([
			{ text: 'Hi' },
			{ usage: { promptTokenCount: 10, candidatesTokenCount: 5 } },
		]);
		const provider = new VertexProvider(gemini as IGeminiModels, makeAnthropicClient() as IAnthropicVertexClient, ['gemini-2.0-flash-001']);

		const chunks = [];
		for await (const chunk of provider.stream(makeRequest({ model: 'gemini-2.0-flash-001' }))) {
			chunks.push(chunk);
		}

		const done = chunks.find(c => c.done);
		assert.ok(done, 'expected a done chunk');
		assert.strictEqual(done.usage?.inputTokens, 10);
		assert.strictEqual(done.usage?.outputTokens, 5);
	});

	test('stream() emits done chunk even when no usageMetadata present', async () => {
		const gemini = makeGeminiModels([{ text: 'Hi' }]); // no usage chunk
		const provider = new VertexProvider(gemini as IGeminiModels, makeAnthropicClient() as IAnthropicVertexClient, ['gemini-2.0-flash-001']);

		const chunks = [];
		for await (const chunk of provider.stream(makeRequest({ model: 'gemini-2.0-flash-001' }))) {
			chunks.push(chunk);
		}

		const done = chunks.find(c => c.done);
		assert.ok(done, 'expected a done chunk even with no usage data');
		assert.strictEqual(done.usage?.inputTokens, 0);
		assert.strictEqual(done.usage?.outputTokens, 0);
	});

	test('stream() routes claude-* models through Anthropic client, not Gemini', async () => {
		const gemini = makeGeminiModels(); // should not be called
		let geminiCalled = false;
		const trackingGemini = {
			...gemini,
			generateContentStream: async (p: unknown) => {
				geminiCalled = true;
				return gemini.generateContentStream(p);
			},
		};

		const anthropicEvents = [
			{ type: 'message_start', message: { usage: { input_tokens: 3, output_tokens: 0 } } },
			{ type: 'content_block_start', index: 0, content_block: { type: 'text', text: '' } },
			{ type: 'content_block_delta', index: 0, delta: { type: 'text_delta', text: 'Hi' } },
			{ type: 'content_block_stop', index: 0 },
			{ type: 'message_delta', delta: { stop_reason: 'end_turn' }, usage: { output_tokens: 2 } },
		];
		const provider = new VertexProvider(trackingGemini as IGeminiModels, makeAnthropicClient(anthropicEvents) as IAnthropicVertexClient, ['claude-sonnet-4-5@20251001']);

		const chunks = [];
		for await (const chunk of provider.stream(makeRequest({ model: 'claude-sonnet-4-5@20251001' }))) {
			chunks.push(chunk);
		}

		assert.strictEqual(geminiCalled, false, 'Gemini client must not be called for claude-* models');
		const textChunks = chunks.filter(c => !c.done);
		assert.deepStrictEqual(textChunks.map(c => c.delta), ['Hi']);
	});

	// --- complete() ---

	test('complete() returns AICompletionResponse for gemini model', async () => {
		const gemini = makeGeminiModels([], 'Hello from Gemini');
		const provider = new VertexProvider(gemini as IGeminiModels, makeAnthropicClient() as IAnthropicVertexClient, ['gemini-2.0-flash-001']);

		const response = await provider.complete(makeRequest({ model: 'gemini-2.0-flash-001' }));

		assert.strictEqual(response.content, 'Hello from Gemini');
		assert.strictEqual(response.model, 'gemini-2.0-flash-001');
		assert.strictEqual(response.inputTokens, 5);
		assert.strictEqual(response.outputTokens, 3);
	});

	test('complete() returns AICompletionResponse for claude model', async () => {
		const anthropic = makeAnthropicClient([], 'Hello from Claude');
		const provider = new VertexProvider(makeGeminiModels() as IGeminiModels, anthropic as IAnthropicVertexClient, ['claude-sonnet-4-5@20251001']);

		const response = await provider.complete(makeRequest({ model: 'claude-sonnet-4-5@20251001' }));

		assert.strictEqual(response.content, 'Hello from Claude');
		assert.strictEqual(response.model, 'claude-sonnet-4-5@20251001');
		assert.strictEqual(response.inputTokens, 5);
		assert.strictEqual(response.outputTokens, 3);
	});

	// --- Claude tool use in stream ---

	test('stream() emits toolUse chunk for claude model with tool call', async () => {
		const anthropicEvents = [
			{ type: 'message_start', message: { usage: { input_tokens: 10, output_tokens: 0 } } },
			{ type: 'content_block_start', index: 0, content_block: { type: 'tool_use', id: 'tool_01', name: 'get_weather' } },
			{ type: 'content_block_delta', index: 0, delta: { type: 'input_json_delta', partial_json: '{"city":' } },
			{ type: 'content_block_delta', index: 0, delta: { type: 'input_json_delta', partial_json: '"Paris"}' } },
			{ type: 'content_block_stop', index: 0 },
			{ type: 'message_delta', delta: { stop_reason: 'tool_use' }, usage: { output_tokens: 20 } },
		];
		const provider = new VertexProvider(makeGeminiModels() as IGeminiModels, makeAnthropicClient(anthropicEvents) as IAnthropicVertexClient, ['claude-sonnet-4-5@20251001']);

		const chunks = [];
		for await (const chunk of provider.stream(makeRequest({ model: 'claude-sonnet-4-5@20251001', tools: [{ name: 'get_weather', description: 'Get weather', inputSchema: {} }] }))) {
			chunks.push(chunk);
		}

		const toolChunk = chunks.find(c => c.toolUse);
		assert.ok(toolChunk, 'expected a toolUse chunk');
		assert.strictEqual(toolChunk.toolUse!.id, 'tool_01');
		assert.strictEqual(toolChunk.toolUse!.name, 'get_weather');
		assert.deepStrictEqual(toolChunk.toolUse!.input, { city: 'Paris' });
	});

	// --- availableModels ---

	test('availableModels returns custom list when provided', () => {
		const provider = new VertexProvider(makeGeminiModels() as IGeminiModels, makeAnthropicClient() as IAnthropicVertexClient, ['gemini-2.0-flash-001', 'claude-sonnet-4-5@20251001']);
		assert.deepStrictEqual(provider.availableModels, ['gemini-2.0-flash-001', 'claude-sonnet-4-5@20251001']);
	});

	test('availableModels returns defaults when no list provided', () => {
		const provider = new VertexProvider(makeGeminiModels() as IGeminiModels, makeAnthropicClient() as IAnthropicVertexClient);
		assert.ok(provider.availableModels.includes('gemini-2.0-flash-001'));
		assert.ok(provider.availableModels.includes('claude-sonnet-4-5@20251001'));
	});
});
