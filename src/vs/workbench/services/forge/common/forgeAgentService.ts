import { createDecorator } from '../../../../platform/instantiation/common/instantiation.js';
import { Event } from '../../../../base/common/event.js';
import { ForgeAgentTask, ForgeAgentTaskOptions, ForgeAgentStep } from './forgeAgentTypes.js';
import { AgentDefinition } from './forgeConfigResolutionTypes.js';

export const IForgeAgentService = createDecorator<IForgeAgentService>('forgeAgentService');

export interface IForgeAgentService {
	readonly _serviceBrand: undefined;

	readonly onDidChangeAgent: Event<ForgeAgentTask>;
	readonly onDidAgentStep: Event<{ agentId: string; step: ForgeAgentStep }>;

	spawnAgent(options: ForgeAgentTaskOptions): Promise<ForgeAgentTask>;
	spawnAgentFromDefinition(agentName: string, taskDescription: string): Promise<ForgeAgentTask>;
	cancelAgent(agentId: string): void;
	getAgent(agentId: string): ForgeAgentTask | undefined;
	getRunningAgents(): ForgeAgentTask[];
	getAllAgents(): ForgeAgentTask[];
	getAvailableDefinitions(): AgentDefinition[];
	isAgentDisabled(agentName: string): boolean;
	toggleAgentDisabled(agentName: string, disabled: boolean): Promise<void>;
}
