import Anthropic from '@anthropic-ai/sdk';
import type { MessageParam, MessageStreamParams, RawContentBlockDeltaEvent, RawContentBlockStartEvent, RawMessageStreamEvent, Tool } from '@anthropic-ai/sdk/resources/messages/messages.js';
import type { AICompletionRequest, AICompletionResponse, AIStreamChunk, AIValidationResult, IAIProvider } from '../common/aiProvider.js';

export class AnthropicProvider implements IAIProvider {

	readonly name = 'anthropic';
	readonly availableModels = ['claude-opus-4-6', 'claude-sonnet-4-6', 'claude-haiku-4-5'];

	constructor(private readonly client: Anthropic) { }

	async complete(request: AICompletionRequest): Promise<AICompletionResponse> {
		const { systemPrompt, messages, model, maxTokens } = this._prepareRequest(request);

		const response = await this.client.messages.create({
			model,
			max_tokens: maxTokens ?? 4096,
			system: systemPrompt,
			messages,
			stream: false,
		});

		const content = response.content
			.filter(block => block.type === 'text')
			.map(block => (block as Anthropic.TextBlock).text)
			.join('');

		return {
			content,
			model: response.model,
			inputTokens: response.usage.input_tokens,
			outputTokens: response.usage.output_tokens,
		};
	}

	async *stream(request: AICompletionRequest): AsyncIterable<AIStreamChunk> {
		const { systemPrompt, messages, model, maxTokens } = this._prepareRequest(request);

		const tools: Tool[] | undefined = request.tools?.map(t => ({
			name: t.name,
			description: t.description,
			input_schema: t.inputSchema as Tool['input_schema'],
		}));

		const streamParams: MessageStreamParams = {
			model,
			max_tokens: maxTokens ?? 4096,
			system: systemPrompt,
			messages,
			...(tools && tools.length > 0 ? { tools } : {}),
		};

		const stream = this.client.messages.stream(streamParams);

		let activeToolBlock: { id: string; name: string; inputJson: string } | undefined;
		let inputTokens = 0;
		let outputTokens = 0;

		for await (const event of stream as AsyncIterable<RawMessageStreamEvent>) {
			if (event.type === 'message_start') {
				const e = event as unknown as { message: { usage: { input_tokens: number; output_tokens: number } } };
				inputTokens = e.message.usage.input_tokens;
				outputTokens = e.message.usage.output_tokens;
			} else if (event.type === 'message_delta') {
				const e = event as unknown as { usage: { output_tokens: number } };
				outputTokens = e.usage.output_tokens;
			} else if (event.type === 'content_block_start') {
				const startEvent = event as RawContentBlockStartEvent;
				if (startEvent.content_block.type === 'tool_use') {
					activeToolBlock = {
						id: startEvent.content_block.id,
						name: startEvent.content_block.name,
						inputJson: '',
					};
				}
			} else if (event.type === 'content_block_delta') {
				const deltaEvent = event as RawContentBlockDeltaEvent;
				if (deltaEvent.delta.type === 'text_delta') {
					yield { delta: deltaEvent.delta.text, done: false };
				} else if (deltaEvent.delta.type === 'input_json_delta' && activeToolBlock) {
					activeToolBlock.inputJson += deltaEvent.delta.partial_json;
				}
			} else if (event.type === 'content_block_stop') {
				if (activeToolBlock) {
					let input: Record<string, unknown>;
					try {
						input = JSON.parse(activeToolBlock.inputJson || '{}') as Record<string, unknown>;
					} catch (e) {
						throw new Error(`Failed to parse tool input JSON for tool '${activeToolBlock.name}': ${(e as Error).message}`);
					}
					yield {
						delta: '',
						done: false,
						toolUse: {
							id: activeToolBlock.id,
							name: activeToolBlock.name,
							input,
						},
					};
					activeToolBlock = undefined;
				}
			}
		}

		yield { delta: '', done: true, usage: { inputTokens, outputTokens } };
	}

	async validateCredentials(): Promise<AIValidationResult> {
		try {
			await this.client.messages.create({
				model: this.availableModels[1],
				max_tokens: 1,
				messages: [{ role: 'user', content: 'hi' }],
			});
			return { valid: true };
		} catch (e) {
			return { valid: false, error: (e as Error).message };
		}
	}

	private _prepareRequest(request: AICompletionRequest): {
		systemPrompt: string | undefined;
		messages: MessageParam[];
		model: string;
		maxTokens: number | undefined;
	} {
		const systemMessages = request.messages.filter(m => m.role === 'system');
		const nonSystemMessages = request.messages.filter(m => m.role !== 'system');

		const systemParts = systemMessages.map(m => m.content);
		if (request.systemPrompt) {
			systemParts.unshift(request.systemPrompt);
		}

		const systemPrompt = systemParts.length > 0 ? systemParts.join('\n\n') : undefined;

		const messages: MessageParam[] = nonSystemMessages.map(m => {
			if (m.role === 'tool_result') {
				if (!m.toolCallId) {
					throw new Error('tool_result message requires toolCallId');
				}
				return {
					role: 'user',
					content: [
						{
							type: 'tool_result' as const,
							tool_use_id: m.toolCallId,
							content: m.content,
						},
					],
				};
			}
			return {
				role: m.role as 'user' | 'assistant',
				content: m.content,
			};
		});

		return { systemPrompt, messages, model: request.model, maxTokens: request.maxTokens };
	}
}
