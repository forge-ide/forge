/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

export interface AIMessage {
	role: 'user' | 'assistant' | 'system';
	content: string;
}

export interface AICompletionRequest {
	messages: AIMessage[];
	model: string;
	maxTokens?: number;
	systemPrompt?: string;
}

export interface AIStreamChunk {
	delta: string;
	done: boolean;
}

export interface AICompletionResponse {
	content: string;
	model: string;
	inputTokens: number;
	outputTokens: number;
}

export interface AIValidationResult {
	valid: boolean;
	error?: string;
}

export interface IAIProvider {
	readonly name: string;
	readonly availableModels: string[];

	complete(request: AICompletionRequest): Promise<AICompletionResponse>;
	stream(request: AICompletionRequest): AsyncIterable<AIStreamChunk>;
	validateCredentials(): Promise<AIValidationResult>;
}
