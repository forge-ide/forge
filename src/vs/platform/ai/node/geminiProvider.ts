/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { GoogleGenerativeAI, type Content } from '@google/generative-ai';
import { AICompletionRequest, AICompletionResponse, AIStreamChunk, AIValidationResult, IAIProvider } from '../common/aiProvider.js';

export class GeminiProvider implements IAIProvider {

	readonly name = 'gemini';
	readonly availableModels = ['gemini-2.0-flash', 'gemini-1.5-pro', 'gemini-1.5-flash'];

	constructor(
		private readonly client: GoogleGenerativeAI,
		private readonly defaultModel: string,
	) { }

	async complete(request: AICompletionRequest): Promise<AICompletionResponse> {
		const { model, contents, systemInstruction } = this._prepareRequest(request);

		const generativeModel = this.client.getGenerativeModel({
			model,
			systemInstruction,
		});

		const result = await generativeModel.generateContent({ contents });
		const response = result.response;

		return {
			content: response.text(),
			model,
			inputTokens: response.usageMetadata?.promptTokenCount ?? 0,
			outputTokens: response.usageMetadata?.candidatesTokenCount ?? 0,
		};
	}

	async *stream(request: AICompletionRequest): AsyncIterable<AIStreamChunk> {
		const { model, contents, systemInstruction } = this._prepareRequest(request);

		const generativeModel = this.client.getGenerativeModel({
			model,
			systemInstruction,
		});

		const result = await generativeModel.generateContentStream({ contents });

		for await (const chunk of result.stream) {
			const text = chunk.text();
			if (text) {
				yield { delta: text, done: false };
			}
		}

		yield { delta: '', done: true };
	}

	async validateCredentials(): Promise<AIValidationResult> {
		try {
			const model = this.client.getGenerativeModel({ model: this.defaultModel });
			await model.generateContent({ contents: [{ role: 'user', parts: [{ text: 'hi' }] }] });
			return { valid: true };
		} catch (e) {
			return { valid: false, error: (e as Error).message };
		}
	}

	private _prepareRequest(request: AICompletionRequest): {
		model: string;
		contents: Content[];
		systemInstruction: string | undefined;
	} {
		const model = request.model || this.defaultModel;

		const systemMessages = request.messages.filter(m => m.role === 'system');
		const nonSystemMessages = request.messages.filter(m => m.role !== 'system');

		const systemParts = systemMessages.map(m => m.content);
		if (request.systemPrompt) {
			systemParts.unshift(request.systemPrompt);
		}
		const systemInstruction = systemParts.length > 0 ? systemParts.join('\n\n') : undefined;

		const contents: Content[] = nonSystemMessages.map(m => ({
			role: m.role === 'assistant' ? 'model' : 'user',
			parts: [{ text: m.content }],
		}));

		return { model, contents, systemInstruction };
	}
}
