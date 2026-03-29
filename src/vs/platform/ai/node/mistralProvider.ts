/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { Mistral } from '@mistralai/mistralai';
import type { Messages } from '@mistralai/mistralai/models/components/chatcompletionrequest.js';
import type { AICompletionRequest, AICompletionResponse, AIStreamChunk, AIValidationResult, IAIProvider } from '../common/aiProvider.js';

export class MistralProvider implements IAIProvider {

	readonly name = 'mistral';
	readonly availableModels = ['mistral-large-latest', 'mistral-small-latest', 'codestral-latest'];

	constructor(private readonly client: Mistral) { }

	async complete(request: AICompletionRequest): Promise<AICompletionResponse> {
		const messages = this._buildMessages(request);

		const response = await this.client.chat.complete({
			model: request.model,
			maxTokens: request.maxTokens,
			messages,
		});

		const choice = response.choices?.[0];
		const content = choice?.message?.content ?? '';
		const text = Array.isArray(content)
			? content.map(c => (c as { text?: string }).text ?? '').join('')
			: (content as string);

		return {
			content: text,
			model: response.model ?? request.model,
			inputTokens: response.usage?.promptTokens ?? 0,
			outputTokens: response.usage?.completionTokens ?? 0,
		};
	}

	async *stream(request: AICompletionRequest): AsyncIterable<AIStreamChunk> {
		const messages = this._buildMessages(request);

		const stream = await this.client.chat.stream({
			model: request.model,
			maxTokens: request.maxTokens,
			messages,
		});

		for await (const event of stream) {
			const delta = event.data.choices[0]?.delta?.content;
			if (typeof delta === 'string' && delta) {
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

	private _buildMessages(request: AICompletionRequest): Messages[] {
		const messages: Messages[] = [];

		if (request.systemPrompt) {
			messages.push({ role: 'system' as const, content: request.systemPrompt });
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
