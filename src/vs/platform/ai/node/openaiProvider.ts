import OpenAI from 'openai';
import type { ChatCompletionMessageParam, ChatCompletionTool } from 'openai/resources/chat/completions/completions.js';
import type { AICompletionRequest, AICompletionResponse, AIStreamChunk, AIValidationResult, IAIProvider } from '../common/aiProvider.js';

export class OpenAIProvider implements IAIProvider {

	readonly name = 'openai';
	readonly availableModels = ['gpt-4o', 'gpt-4o-mini', 'gpt-4-turbo', 'o1', 'o1-mini'];

	constructor(private readonly client: OpenAI) { }

	async complete(request: AICompletionRequest): Promise<AICompletionResponse> {
		const messages = this._buildMessages(request);

		const response = await this.client.chat.completions.create({
			model: request.model,
			max_tokens: request.maxTokens,
			messages,
			stream: false,
		});

		const choice = response.choices[0];
		return {
			content: choice.message.content ?? '',
			model: response.model,
			inputTokens: response.usage?.prompt_tokens ?? 0,
			outputTokens: response.usage?.completion_tokens ?? 0,
		};
	}

	async *stream(request: AICompletionRequest): AsyncIterable<AIStreamChunk> {
		const messages = this._buildMessages(request);

		const tools: ChatCompletionTool[] | undefined = request.tools?.map(t => ({
			type: 'function' as const,
			function: {
				name: t.name,
				description: t.description,
				parameters: t.inputSchema as Record<string, unknown>,
			},
		}));

		const stream = await this.client.chat.completions.create({
			model: request.model,
			max_tokens: request.maxTokens,
			messages,
			stream: true,
			stream_options: { include_usage: true },
			...(tools && tools.length > 0 ? { tools } : {}),
		});

		const pendingToolCalls = new Map<number, { id: string; name: string; args: string }>();
		let inputTokens = 0;
		let outputTokens = 0;

		for await (const chunk of stream) {
			if (chunk.usage) {
				inputTokens = chunk.usage.prompt_tokens ?? 0;
				outputTokens = chunk.usage.completion_tokens ?? 0;
			}

			const choice = chunk.choices[0];
			if (!choice) {
				continue;
			}

			const delta = choice.delta;

			if (delta.content) {
				yield { delta: delta.content, done: false };
			}

			if (delta.tool_calls) {
				for (const tc of delta.tool_calls) {
					const existing = pendingToolCalls.get(tc.index);
					if (existing) {
						existing.args += tc.function?.arguments ?? '';
					} else {
						pendingToolCalls.set(tc.index, {
							id: tc.id ?? '',
							name: tc.function?.name ?? '',
							args: tc.function?.arguments ?? '',
						});
					}
				}
			}

			if (choice.finish_reason === 'tool_calls') {
				for (const [, tc] of pendingToolCalls) {
					let input: Record<string, unknown>;
					try {
						input = JSON.parse(tc.args || '{}') as Record<string, unknown>;
					} catch (e) {
						throw new Error(`Failed to parse tool arguments JSON for tool '${tc.name}': ${(e as Error).message}`);
					}
					yield {
						delta: '',
						done: false,
						toolUse: {
							id: tc.id,
							name: tc.name,
							input,
						},
					};
				}
				pendingToolCalls.clear();
			}
		}

		yield { delta: '', done: true, usage: { inputTokens, outputTokens } };
	}

	async validateCredentials(): Promise<AIValidationResult> {
		try {
			await this.client.models.list();
			return { valid: true };
		} catch (e) {
			return { valid: false, error: (e as Error).message };
		}
	}

	private _buildMessages(request: AICompletionRequest): ChatCompletionMessageParam[] {
		const messages: ChatCompletionMessageParam[] = [];

		if (request.systemPrompt) {
			messages.push({ role: 'system', content: request.systemPrompt });
		}

		for (const m of request.messages) {
			if (m.role === 'system') {
				continue;
			}
			if (m.role === 'user') {
				messages.push({ role: 'user' as const, content: m.content });
			} else if (m.role === 'tool_result') {
				if (!m.toolCallId) {
					throw new Error('tool_result message requires toolCallId');
				}
				messages.push({
					role: 'tool' as const,
					tool_call_id: m.toolCallId,
					content: m.content,
				});
			} else {
				messages.push({ role: 'assistant' as const, content: m.content });
			}
		}

		return messages;
	}
}
