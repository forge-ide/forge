/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

export interface AIMessage {
	readonly role: 'user' | 'assistant' | 'system' | 'tool_result';
	readonly content: string;
	readonly toolCallId?: string;
}

export interface AIToolDefinition {
	readonly name: string;
	readonly description: string;
	readonly inputSchema: Record<string, unknown>;
}

export interface AIToolUse {
	readonly id: string;
	readonly name: string;
	readonly input: Record<string, unknown>;
}

export interface AICompletionRequest {
	readonly messages: AIMessage[];
	readonly model: string;
	readonly maxTokens?: number;
	readonly systemPrompt?: string;
	readonly tools?: AIToolDefinition[];
}

export interface AIStreamChunk {
	readonly delta: string;
	readonly done: boolean;
	readonly toolUse?: AIToolUse;
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
