import { Disposable } from '../../../../base/common/lifecycle.js';
import { Emitter, Event } from '../../../../base/common/event.js';
import { ILogService } from '../../../../platform/log/common/log.js';
import { IAIProviderService } from '../../../../platform/ai/common/aiProviderService.js';
import { AIMessage } from '../../../../platform/ai/common/aiProvider.js';
import { IForgeMcpService } from '../common/forgeMcpService.js';
import { IForgeConfigResolutionService } from '../common/forgeConfigResolution.js';
import { IForgeAgentService } from '../common/forgeAgentService.js';
import { AgentDefinition } from '../common/forgeConfigResolutionTypes.js';
import {
	ForgeAgentTask,
	ForgeAgentTaskOptions,
	ForgeAgentStep,
	ForgeAgentStatus,
	createAgentTask,
	createAgentTaskFromDefinition,
	createAgentStep
} from '../common/forgeAgentTypes.js';

export class ForgeAgentService extends Disposable implements IForgeAgentService {
	declare readonly _serviceBrand: undefined;

	private readonly agents = new Map<string, ForgeAgentTask>();

	private readonly _onDidChangeAgent = this._register(new Emitter<ForgeAgentTask>());
	readonly onDidChangeAgent: Event<ForgeAgentTask> = this._onDidChangeAgent.event;

	private readonly _onDidAgentStep = this._register(new Emitter<{ agentId: string; step: ForgeAgentStep }>());
	readonly onDidAgentStep: Event<{ agentId: string; step: ForgeAgentStep }> = this._onDidAgentStep.event;

	constructor(
		@IAIProviderService private readonly aiProviderService: IAIProviderService,
		@IForgeMcpService private readonly forgeMcpService: IForgeMcpService,
		@IForgeConfigResolutionService private readonly configResolutionService: IForgeConfigResolutionService,
		@ILogService private readonly logService: ILogService
	) {
		super();
	}

	async spawnAgent(options: ForgeAgentTaskOptions): Promise<ForgeAgentTask> {
		const task = createAgentTask(options);
		this.agents.set(task.id, task);
		this._onDidChangeAgent.fire(task);

		this.runAgentLoop(task).catch(err => {
			this.logService.error(`[ForgeAgentService] Agent ${task.id} failed:`, err);
		});

		return task;
	}

	async spawnAgentFromDefinition(agentName: string, taskDescription: string): Promise<ForgeAgentTask> {
		const definitions = this.getAvailableDefinitions();
		const definition = definitions.find(d => d.name === agentName);
		if (!definition) {
			throw new Error(`Agent definition "${agentName}" not found`);
		}
		const task = createAgentTaskFromDefinition(definition, taskDescription);
		this.agents.set(task.id, task);
		this._onDidChangeAgent.fire(task);

		this.runAgentLoop(task).catch(err => {
			this.logService.error(`[ForgeAgentService] Agent ${task.id} failed:`, err);
		});

		return task;
	}

	cancelAgent(agentId: string): void {
		const task = this.agents.get(agentId);
		if (task && task.status === ForgeAgentStatus.Running) {
			task.status = ForgeAgentStatus.Error;
			task.error = 'Cancelled by user';
			task.completedAt = Date.now();
			this._onDidChangeAgent.fire(task);
		}
	}

	getAgent(agentId: string): ForgeAgentTask | undefined {
		return this.agents.get(agentId);
	}

	getRunningAgents(): ForgeAgentTask[] {
		return [...this.agents.values()].filter(
			a => a.status === ForgeAgentStatus.Running || a.status === ForgeAgentStatus.Queued
		);
	}

	getAllAgents(): ForgeAgentTask[] {
		return [...this.agents.values()];
	}

	getAvailableDefinitions(): AgentDefinition[] {
		return this.configResolutionService.getCached()?.agents ?? [];
	}

	isAgentDisabled(agentName: string): boolean {
		return this.configResolutionService.getCached()?.disabled?.agents?.includes(agentName) ?? false;
	}

	async toggleAgentDisabled(agentName: string, disabled: boolean): Promise<void> {
		await this.configResolutionService.setAgentDisabled(agentName, disabled);
	}

	private async runAgentLoop(task: ForgeAgentTask): Promise<void> {
		task.status = ForgeAgentStatus.Running;
		task.startedAt = Date.now();
		this._onDidChangeAgent.fire(task);

		const provider = this.aiProviderService.getProvider(task.providerName);
		if (!provider) {
			task.status = ForgeAgentStatus.Error;
			task.error = `Provider "${task.providerName}" not found`;
			task.completedAt = Date.now();
			this._onDidChangeAgent.fire(task);
			return;
		}

		const mcpTools = await this.forgeMcpService.listTools();
		const messages: AIMessage[] = [
			{ role: 'system', content: task.systemPrompt },
			{ role: 'user', content: task.taskDescription }
		];

		while (task.currentTurn < task.maxTurns && task.status === ForgeAgentStatus.Running) {
			task.currentTurn++;

			let fullContent = '';
			let toolUse: { id: string; name: string; input: Record<string, unknown> } | undefined;

			try {
				for await (const chunk of provider.stream({
					model: task.model,
					messages,
					tools: mcpTools.length > 0 ? mcpTools : undefined
				})) {
					if (chunk.toolUse) {
						toolUse = chunk.toolUse;
					} else {
						fullContent += chunk.delta;
					}
					if (chunk.done) {
						break;
					}
				}
			} catch (err: unknown) {
				const errMsg = err instanceof Error ? err.message : String(err);
				task.status = ForgeAgentStatus.Error;
				task.error = errMsg;
				task.completedAt = Date.now();
				this._onDidChangeAgent.fire(task);
				return;
			}

			// Check if cancelled during stream
			if (task.status !== ForgeAgentStatus.Running) {
				return;
			}

			if (!toolUse) {
				task.status = ForgeAgentStatus.Completed;
				task.result = fullContent;
				task.completedAt = Date.now();
				this._onDidChangeAgent.fire(task);
				return;
			}

			const step = createAgentStep(toolUse.id, toolUse.name, toolUse.input);
			step.status = 'running';
			task.steps.push(step);
			this._onDidAgentStep.fire({ agentId: task.id, step });

			try {
				const result = await this.forgeMcpService.callTool(toolUse.name, toolUse.input);

				// Check if cancelled during tool call
				if (task.status !== ForgeAgentStatus.Running) {
					return;
				}

				step.status = 'completed';
				step.result = result.content;
				step.completedAt = Date.now();
				this._onDidAgentStep.fire({ agentId: task.id, step });

				messages.push({ role: 'assistant', content: fullContent });
				messages.push({
					role: 'tool_result',
					content: result.content,
					toolCallId: toolUse.id
				});
			} catch (err: unknown) {
				const errMsg = err instanceof Error ? err.message : String(err);
				step.status = 'error';
				step.error = errMsg;
				step.completedAt = Date.now();
				this._onDidAgentStep.fire({ agentId: task.id, step });

				task.status = ForgeAgentStatus.Error;
				task.error = `Tool "${toolUse.name}" failed: ${errMsg}`;
				task.completedAt = Date.now();
				this._onDidChangeAgent.fire(task);
				return;
			}

			this._onDidChangeAgent.fire(task);
		}

		if (task.status === ForgeAgentStatus.Running) {
			task.status = ForgeAgentStatus.MaxTurnsReached;
			task.completedAt = Date.now();
			this._onDidChangeAgent.fire(task);
		}
	}
}
