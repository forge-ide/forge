/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import Anthropic from '@anthropic-ai/sdk';
import type { MessageParam } from '@anthropic-ai/sdk/resources/messages.js';
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

		const stream = this.client.messages.stream({
			model,
			max_tokens: maxTokens ?? 4096,
			system: systemPrompt,
			messages,
		});

		for await (const event of stream) {
			if (
				event.type === 'content_block_delta' &&
				event.delta.type === 'text_delta'
			) {
				yield { delta: event.delta.text, done: false };
			}
		}

		yield { delta: '', done: true };
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

		const messages: MessageParam[] = nonSystemMessages.map(m => ({
			role: m.role as 'user' | 'assistant',
			content: m.content,
		}));

		return { systemPrompt, messages, model: request.model, maxTokens: request.maxTokens };
	}
}
