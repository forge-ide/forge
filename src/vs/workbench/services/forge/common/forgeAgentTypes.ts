import { AgentDefinition } from './forgeConfigResolutionTypes.js';

export const MAX_TURNS = 20;

export enum ForgeAgentStatus {
	Queued = 'queued',
	Running = 'running',
	Completed = 'completed',
	Error = 'error',
	MaxTurnsReached = 'max_turns_reached'
}

export interface ForgeAgentStep {
	readonly toolCallId: string;
	readonly toolName: string;
	readonly input: Record<string, unknown>;
	status: 'pending' | 'running' | 'completed' | 'error';
	readonly startedAt: number;
	result?: string;
	error?: string;
	completedAt?: number;
}

export interface ForgeAgentTask {
	readonly id: string;
	readonly name: string;
	readonly systemPrompt: string;
	readonly taskDescription: string;
	readonly providerName: string;
	readonly model: string;
	readonly maxTurns: number;
	readonly allowedTools?: string[];
	status: ForgeAgentStatus;
	currentTurn: number;
	steps: ForgeAgentStep[];
	result?: string;
	error?: string;
	startedAt?: number;
	completedAt?: number;
	tokenCount?: { input: number; output: number };
}

export interface ForgeAgentTaskOptions {
	readonly name: string;
	readonly systemPrompt: string;
	readonly taskDescription: string;
	readonly providerName: string;
	readonly model: string;
	readonly maxTurns?: number;
	readonly allowedTools?: string[];
}

let agentCounter = 0;

export function createAgentTask(options: ForgeAgentTaskOptions): ForgeAgentTask {
	return {
		id: `agent_${++agentCounter}_${Date.now()}`,
		name: options.name,
		systemPrompt: options.systemPrompt,
		taskDescription: options.taskDescription,
		providerName: options.providerName,
		model: options.model,
		maxTurns: Math.min(options.maxTurns ?? MAX_TURNS, MAX_TURNS),
		allowedTools: options.allowedTools,
		status: ForgeAgentStatus.Queued,
		currentTurn: 0,
		steps: []
	};
}

export function createAgentTaskFromDefinition(
	definition: AgentDefinition,
	taskDescription: string
): ForgeAgentTask {
	return createAgentTask({
		name: definition.name,
		systemPrompt: definition.systemPrompt,
		taskDescription,
		providerName: definition.provider ?? '',
		model: definition.model ?? '',
		maxTurns: definition.maxTurns,
		allowedTools: definition.tools,
	});
}

export function createAgentStep(
	toolCallId: string,
	toolName: string,
	input: Record<string, unknown>
): ForgeAgentStep {
	return {
		toolCallId,
		toolName,
		input,
		status: 'pending',
		startedAt: Date.now()
	};
}
