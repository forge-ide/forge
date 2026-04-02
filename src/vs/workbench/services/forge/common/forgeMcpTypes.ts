/**
 * Runtime MCP types for tool call tracking and server status.
 * Config types live in forgeConfigResolutionTypes.ts.
 */

export const enum ForgeMcpServerStatus {
	Disconnected = 'disconnected',
	Connecting = 'connecting',
	Connected = 'connected',
	Error = 'error'
}

export interface ForgeMcpToolRecord {
	readonly callId: string;
	readonly toolName: string;
	readonly input: Record<string, unknown>;
	readonly serverName: string;
	readonly status: 'pending' | 'running' | 'completed' | 'error';
	readonly startedAt: number;
}

export interface ForgeMcpToolResultRecord {
	readonly callId: string;
	readonly content: string;
	readonly isError: boolean;
	readonly completedAt: number;
	readonly durationMs?: number;
}

export function createToolRecord(
	callId: string,
	toolName: string,
	input: Record<string, unknown>,
	serverName: string
): ForgeMcpToolRecord {
	return {
		callId,
		toolName,
		input,
		serverName,
		status: 'pending',
		startedAt: Date.now()
	};
}

export function createToolResultRecord(
	callId: string,
	content: string,
	isError: boolean
): ForgeMcpToolResultRecord {
	return {
		callId,
		content,
		isError,
		completedAt: Date.now()
	};
}
