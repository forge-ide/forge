export interface ForgeChatTextPart {
	readonly type: 'text';
	readonly content: string;
}

export interface ForgeChatToolCallPart {
	readonly type: 'tool_call';
	readonly callId: string;
	readonly toolName: string;
	readonly input: Record<string, unknown>;
	readonly serverName: string;
	readonly status: 'pending' | 'running' | 'completed' | 'error';
}

export interface ForgeChatToolResultPart {
	readonly type: 'tool_result';
	readonly callId: string;
	readonly content: string;
	readonly isError: boolean;
	readonly durationMs?: number;
}

export interface ForgeChatAgentProgressPart {
	readonly type: 'agent_progress';
	readonly agentId: string;
	readonly agentName: string;
	readonly status: 'queued' | 'running' | 'completed' | 'error';
	readonly currentStep?: number;
	readonly totalSteps?: number;
	readonly stepLabel?: string;
}

export type ForgeChatContentPart =
	| ForgeChatTextPart
	| ForgeChatToolCallPart
	| ForgeChatToolResultPart
	| ForgeChatAgentProgressPart;

export interface ForgeAssistantMessage {
	readonly role: 'assistant';
	readonly parts: ForgeChatContentPart[];
}

export function isTextPart(part: ForgeChatContentPart): part is ForgeChatTextPart {
	return part.type === 'text';
}

export function isToolCallPart(part: ForgeChatContentPart): part is ForgeChatToolCallPart {
	return part.type === 'tool_call';
}

export function isToolResultPart(part: ForgeChatContentPart): part is ForgeChatToolResultPart {
	return part.type === 'tool_result';
}

export function isAgentProgressPart(part: ForgeChatContentPart): part is ForgeChatAgentProgressPart {
	return part.type === 'agent_progress';
}
