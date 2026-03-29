/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import OpenAI from 'openai';
import type { ChatCompletionMessageParam } from 'openai/resources/chat/completions.js';
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

		const stream = await this.client.chat.completions.create({
			model: request.model,
			max_tokens: request.maxTokens,
			messages,
			stream: true,
		});

		for await (const chunk of stream) {
			const delta = chunk.choices[0]?.delta?.content;
			if (delta) {
				yield { delta, done: false };
			}
		}

		yield { delta: '', done: true };
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
			} else {
				messages.push({ role: 'assistant' as const, content: m.content });
			}
		}

		return messages;
	}
}
