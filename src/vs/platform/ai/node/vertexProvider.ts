import type { AICompletionRequest, AICompletionResponse, AIStreamChunk, AIToolDefinition, AIValidationResult, IAIProvider } from '../common/aiProvider.js';

const DEFAULT_MODELS = [
	'gemini-2.0-flash-001',
	'gemini-1.5-pro-002',
	'gemini-1.5-flash-002',
	'claude-sonnet-4-5@20251001',
	'claude-haiku-4-5@20251001',
];

// Shape of @google/genai `ai.models` sub-object used for Vertex inference.
export interface IGeminiModels {
	generateContentStream(params: unknown): Promise<AsyncIterable<{
		candidates?: Array<{ content?: { parts?: Array<{ text?: string; functionCall?: { name: string; args: unknown } }> } }>;
		usageMetadata?: { promptTokenCount?: number; candidatesTokenCount?: number };
	}>>;
	generateContent(params: unknown): Promise<{
		candidates?: Array<{ content?: { parts?: Array<{ text?: string; functionCall?: { name: string; args: unknown } }> } }>;
		usageMetadata?: { promptTokenCount?: number; candidatesTokenCount?: number };
	}>;
}

// Shape of AnthropicVertex client used for Claude-on-Vertex inference.
export interface IAnthropicVertexClient {
	messages: {
		stream(params: unknown): AsyncIterable<unknown>;
		create(params: unknown): Promise<{
			content: Array<{ type: string; text?: string; id?: string; name?: string; input?: unknown }>;
			usage: { input_tokens: number; output_tokens: number };
			model: string;
		}>;
	};
}

export class VertexProvider implements IAIProvider {

	readonly name = 'vertex';
	readonly availableModels: string[];

	constructor(
		private readonly geminiModels: IGeminiModels,
		private readonly anthropicClient: IAnthropicVertexClient,
		models: string[] = DEFAULT_MODELS,
	) {
		this.availableModels = models;
	}

	async *stream(request: AICompletionRequest): AsyncIterable<AIStreamChunk> {
		if (request.model.startsWith('claude-')) {
			yield* this._streamClaude(request);
		} else {
			yield* this._streamGemini(request);
		}
	}

	async complete(request: AICompletionRequest): Promise<AICompletionResponse> {
		if (request.model.startsWith('claude-')) {
			return this._completeClaude(request);
		}
		return this._completeGemini(request);
	}

	async validateCredentials(): Promise<AIValidationResult> {
		try {
			await this.geminiModels.generateContent({
				model: 'gemini-2.0-flash-001',
				contents: [{ role: 'user', parts: [{ text: 'hi' }] }],
				config: { maxOutputTokens: 1 },
			});
			return { valid: true };
		} catch (err) {
			return { valid: false, error: err instanceof Error ? err.message : String(err) };
		}
	}

	// --- Gemini path ---

	private async *_streamGemini(request: AICompletionRequest): AsyncIterable<AIStreamChunk> {
		const { contents, systemInstruction } = this._prepareGeminiContents(request);

		const stream = await this.geminiModels.generateContentStream({
			model: request.model,
			contents,
			config: {
				...(systemInstruction ? { systemInstruction } : {}),
				...(request.maxTokens ? { maxOutputTokens: request.maxTokens } : {}),
				...(request.tools?.length ? { tools: [{ functionDeclarations: request.tools.map(this._toGeminiFunctionDeclaration) }] } : {}),
			},
		});

		for await (const chunk of stream) {
			const parts = chunk.candidates?.[0]?.content?.parts ?? [];
			for (const part of parts) {
				if (part.text) {
					yield { delta: part.text, done: false };
				}
			}
			if (chunk.usageMetadata) {
				yield {
					delta: '',
					done: true,
					usage: {
						inputTokens: chunk.usageMetadata.promptTokenCount ?? 0,
						outputTokens: chunk.usageMetadata.candidatesTokenCount ?? 0,
					},
				};
			}
		}
	}

	private async _completeGemini(request: AICompletionRequest): Promise<AICompletionResponse> {
		const { contents, systemInstruction } = this._prepareGeminiContents(request);

		const result = await this.geminiModels.generateContent({
			model: request.model,
			contents,
			config: {
				...(systemInstruction ? { systemInstruction } : {}),
				...(request.maxTokens ? { maxOutputTokens: request.maxTokens } : {}),
				...(request.tools?.length ? { tools: [{ functionDeclarations: request.tools.map(this._toGeminiFunctionDeclaration) }] } : {}),
			},
		});

		const parts = result.candidates?.[0]?.content?.parts ?? [];
		const content = parts.filter(p => p.text).map(p => p.text as string).join('');

		return {
			content,
			model: request.model,
			inputTokens: result.usageMetadata?.promptTokenCount ?? 0,
			outputTokens: result.usageMetadata?.candidatesTokenCount ?? 0,
		};
	}

	private _prepareGeminiContents(request: AICompletionRequest): {
		contents: Array<{ role: string; parts: Array<{ text: string }> }>;
		systemInstruction?: string;
	} {
		const systemParts: string[] = [];
		if (request.systemPrompt) { systemParts.push(request.systemPrompt); }
		systemParts.push(...request.messages.filter(m => m.role === 'system').map(m => m.content));
		const systemInstruction = systemParts.length > 0 ? systemParts.join('\n\n') : undefined;

		const contents = request.messages
			.filter(m => m.role !== 'system')
			.map(m => ({
				role: m.role === 'assistant' ? 'model' : 'user',
				parts: [{ text: m.content }],
			}));

		return { contents, systemInstruction };
	}

	private _toGeminiFunctionDeclaration(tool: AIToolDefinition): unknown {
		return {
			name: tool.name,
			description: tool.description,
			parameters: tool.inputSchema,
		};
	}

	// --- Claude path ---

	private async *_streamClaude(request: AICompletionRequest): AsyncIterable<AIStreamChunk> {
		const { messages, systemPrompt } = this._prepareAnthropicMessages(request);

		const params: Record<string, unknown> = {
			model: request.model,
			messages,
			max_tokens: request.maxTokens ?? 4096,
			...(systemPrompt ? { system: systemPrompt } : {}),
			...(request.tools?.length ? { tools: request.tools.map(this._toAnthropicTool) } : {}),
		};

		let inputTokens = 0;
		let outputTokens = 0;
		let activeToolBlock: { id: string; name: string; inputJson: string } | undefined;

		for await (const event of this.anthropicClient.messages.stream(params) as AsyncIterable<Record<string, unknown>>) {
			if (event['type'] === 'message_start') {
				const msg = event['message'] as { usage: { input_tokens: number } };
				inputTokens = msg.usage.input_tokens;
			} else if (event['type'] === 'message_delta') {
				const usage = event['usage'] as { output_tokens: number };
				outputTokens = usage.output_tokens;
			} else if (event['type'] === 'content_block_start') {
				const block = event['content_block'] as { type: string; id?: string; name?: string };
				if (block.type === 'tool_use') {
					activeToolBlock = { id: block.id!, name: block.name!, inputJson: '' };
				}
			} else if (event['type'] === 'content_block_delta') {
				const delta = event['delta'] as { type: string; text?: string; partial_json?: string };
				if (delta.type === 'text_delta') {
					yield { delta: delta.text!, done: false };
				} else if (delta.type === 'input_json_delta' && activeToolBlock) {
					activeToolBlock.inputJson += delta.partial_json;
				}
			} else if (event['type'] === 'content_block_stop' && activeToolBlock) {
				let input: Record<string, unknown>;
				try {
					input = JSON.parse(activeToolBlock.inputJson || '{}') as Record<string, unknown>;
				} catch (e) {
					throw new Error(`Failed to parse tool input JSON for '${activeToolBlock.name}': ${(e as Error).message}`);
				}
				yield { delta: '', done: false, toolUse: { id: activeToolBlock.id, name: activeToolBlock.name, input } };
				activeToolBlock = undefined;
			}
		}

		yield { delta: '', done: true, usage: { inputTokens, outputTokens } };
	}

	private async _completeClaude(request: AICompletionRequest): Promise<AICompletionResponse> {
		const { messages, systemPrompt } = this._prepareAnthropicMessages(request);

		const params: Record<string, unknown> = {
			model: request.model,
			messages,
			max_tokens: request.maxTokens ?? 4096,
			...(systemPrompt ? { system: systemPrompt } : {}),
			...(request.tools?.length ? { tools: request.tools.map(this._toAnthropicTool) } : {}),
		};

		const result = await this.anthropicClient.messages.create(params);

		const content = result.content
			.filter(b => b.type === 'text')
			.map(b => b.text!)
			.join('');

		return {
			content,
			model: result.model,
			inputTokens: result.usage.input_tokens,
			outputTokens: result.usage.output_tokens,
		};
	}

	private _prepareAnthropicMessages(request: AICompletionRequest): {
		messages: Array<{ role: string; content: unknown }>;
		systemPrompt?: string;
	} {
		const systemParts: string[] = [];
		if (request.systemPrompt) { systemParts.push(request.systemPrompt); }
		systemParts.push(...request.messages.filter(m => m.role === 'system').map(m => m.content));
		const systemPrompt = systemParts.length > 0 ? systemParts.join('\n\n') : undefined;

		const messages = request.messages
			.filter(m => m.role !== 'system')
			.map(m => {
				if (m.role === 'tool_result') {
					if (!m.toolCallId) { throw new Error('tool_result message requires toolCallId'); }
					return {
						role: 'user',
						content: [{ type: 'tool_result', tool_use_id: m.toolCallId, content: m.content }],
					};
				}
				return { role: m.role as string, content: m.content };
			});

		return { messages, systemPrompt };
	}

	private _toAnthropicTool(tool: AIToolDefinition): unknown {
		return {
			name: tool.name,
			description: tool.description,
			input_schema: tool.inputSchema,
		};
	}
}
