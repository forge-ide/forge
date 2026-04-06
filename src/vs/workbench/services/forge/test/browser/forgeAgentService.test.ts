import assert from 'assert';
import { Emitter } from '../../../../../base/common/event.js';
import { DisposableStore } from '../../../../../base/common/lifecycle.js';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import { ForgeAgentService } from '../../browser/forgeAgentService.js';
import { ForgeAgentStatus } from '../../common/forgeAgentTypes.js';
import { NullLogService } from '../../../../../platform/log/common/log.js';
import { AICompletionRequest, AIStreamChunk, AIToolDefinition } from '../../../../../platform/ai/common/aiProvider.js';
import { ForgeMcpServerStatusEntry } from '../../common/forgeMcpService.js';
import { ResolvedConfig } from '../../common/forgeConfigResolutionTypes.js';

suite('ForgeAgentService', () => {
	let disposables: DisposableStore;

	setup(() => {
		disposables = new DisposableStore();
	});

	teardown(() => {
		disposables.dispose();
	});

	ensureNoDisposablesAreLeakedInTestSuite();

	function createMockProvider(responses: AIStreamChunk[][]) {
		let callIndex = 0;
		return {
			name: 'mock',
			availableModels: ['mock-model'],
			async *stream() {
				const chunks = responses[callIndex++] ?? [{ delta: 'done', done: true }];
				for (const chunk of chunks) {
					yield chunk;
				}
			},
			async complete() { return { content: '', model: '', inputTokens: 0, outputTokens: 0 }; },
			async validateCredentials() { return { valid: true }; }
		};
	}

	function createMockAIProviderService(provider: ReturnType<typeof createMockProvider>, ds: DisposableStore) {
		return {
			_serviceBrand: undefined as undefined,
			getProvider: () => provider,
			listProviders: () => ['mock'],
			has: () => true,
			registerProvider: () => { },
			unregisterProvider: () => { },
			getDefaultProviderName: () => 'mock' as string | undefined,
			setDefaultProviderName: () => { },
			onDidChangeProviders: ds.add(new Emitter<string[]>()).event
		};
	}

	function createMockMcpService(ds: DisposableStore) {
		return {
			_serviceBrand: undefined as undefined,
			onDidChangeTools: ds.add(new Emitter<void>()).event,
			onDidChangeServerStatus: ds.add(new Emitter<ForgeMcpServerStatusEntry>()).event,
			listTools: async (): Promise<AIToolDefinition[]> => [{
				name: 'read_file',
				description: 'Read a file',
				inputSchema: { type: 'object', properties: { path: { type: 'string' } } }
			}],
			callTool: async (name: string, input: Record<string, unknown>) => ({
				callId: `${name}_${Date.now()}`,
				content: `contents of ${(input as { path: string }).path}`,
				isError: false,
				completedAt: Date.now()
			}),
			getServerStatuses: () => [],
			isServerDisabled: () => false,
			toggleServerDisabled: async () => { }
		};
	}

	function createMockConfigResolutionService(ds: DisposableStore) {
		return {
			_serviceBrand: undefined as undefined,
			onDidChangeResolved: ds.add(new Emitter<ResolvedConfig>()).event,
			resolve: async () => ({ mcpServers: [], agents: [], skills: [], disabled: { mcpServers: [], agents: [] } }),
			getCached: () => ({ mcpServers: [], agents: [], skills: [], disabled: { mcpServers: [], agents: [] } }),
			setMcpServerDisabled: async () => { },
			setAgentDisabled: async () => { }
		};
	}

	test('agent completes simple text response', async () => {
		const provider = createMockProvider([
			[{ delta: 'Task complete.', done: true }]
		]);
		const service = disposables.add(new ForgeAgentService(
			createMockAIProviderService(provider, disposables) as never,
			createMockMcpService(disposables) as never,
			createMockConfigResolutionService(disposables) as never,
			new NullLogService()
		));

		const task = await service.spawnAgent({
			name: 'Test Agent',
			systemPrompt: 'You are a test agent.',
			taskDescription: 'Say hello.',
			providerName: 'mock',
			model: 'mock-model'
		});

		// Wait for completion
		await new Promise(resolve => setTimeout(resolve, 100));

		const completed = service.getAgent(task.id);
		assert.ok(completed);
		assert.strictEqual(completed!.status, ForgeAgentStatus.Completed);
	});

	test('agent executes tool call loop', async () => {
		const provider = createMockProvider([
			// Turn 1: tool call
			[
				{ delta: '', done: false, toolUse: { id: 'call_1', name: 'read_file', input: { path: '/tmp/a.txt' } } },
				{ delta: '', done: true }
			],
			// Turn 2: text response after tool result
			[{ delta: 'File contents received. Done.', done: true }]
		]);

		const service = disposables.add(new ForgeAgentService(
			createMockAIProviderService(provider, disposables) as never,
			createMockMcpService(disposables) as never,
			createMockConfigResolutionService(disposables) as never,
			new NullLogService()
		));

		const task = await service.spawnAgent({
			name: 'Tool Agent',
			systemPrompt: 'You read files.',
			taskDescription: 'Read /tmp/a.txt',
			providerName: 'mock',
			model: 'mock-model'
		});

		await new Promise(resolve => setTimeout(resolve, 200));

		const completed = service.getAgent(task.id);
		assert.ok(completed);
		assert.strictEqual(completed!.status, ForgeAgentStatus.Completed);
		assert.strictEqual(completed!.steps.length, 1);
		assert.strictEqual(completed!.steps[0].toolName, 'read_file');
	});

	test('agent respects maxTurns', async () => {
		// Always returns a tool call — should stop at max turns
		const infiniteToolCalls: AIStreamChunk[][] = Array(25).fill([
			{ delta: '', done: false, toolUse: { id: 'call_x', name: 'read_file', input: { path: '/a' } } },
			{ delta: '', done: true }
		]);

		const provider = createMockProvider(infiniteToolCalls);

		const service = disposables.add(new ForgeAgentService(
			createMockAIProviderService(provider, disposables) as never,
			createMockMcpService(disposables) as never,
			createMockConfigResolutionService(disposables) as never,
			new NullLogService()
		));

		const task = await service.spawnAgent({
			name: 'Loop Agent',
			systemPrompt: 'Keep reading.',
			taskDescription: 'Read forever.',
			providerName: 'mock',
			model: 'mock-model',
			maxTurns: 3
		});

		await new Promise(resolve => setTimeout(resolve, 500));

		const completed = service.getAgent(task.id);
		assert.ok(completed);
		assert.strictEqual(completed!.status, ForgeAgentStatus.MaxTurnsReached);
		assert.ok(completed!.currentTurn <= 3);
	});

	test('agent result field populated after text completion', async () => {
		const provider = createMockProvider([
			[{ delta: 'The answer is 42.', done: true }]
		]);
		const service = disposables.add(new ForgeAgentService(
			createMockAIProviderService(provider, disposables) as never,
			createMockMcpService(disposables) as never,
			createMockConfigResolutionService(disposables) as never,
			new NullLogService()
		));

		const task = await service.spawnAgent({
			name: 'Result Agent', systemPrompt: 'Answer questions.', taskDescription: 'What is 6*7?',
			providerName: 'mock', model: 'mock-model'
		});
		await new Promise(resolve => setTimeout(resolve, 100));

		const completed = service.getAgent(task.id);
		assert.ok(completed);
		assert.strictEqual(completed!.result, 'The answer is 42.');
	});

	test('agent completedAt is set after completion', async () => {
		const before = Date.now();
		const provider = createMockProvider([[{ delta: 'done', done: true }]]);
		const service = disposables.add(new ForgeAgentService(
			createMockAIProviderService(provider, disposables) as never,
			createMockMcpService(disposables) as never,
			createMockConfigResolutionService(disposables) as never,
			new NullLogService()
		));

		const task = await service.spawnAgent({
			name: 'Agent', systemPrompt: 'test', taskDescription: 'test',
			providerName: 'mock', model: 'mock-model'
		});
		await new Promise(resolve => setTimeout(resolve, 100));

		const completed = service.getAgent(task.id);
		assert.ok(completed?.completedAt);
		assert.ok(completed!.completedAt! >= before);
	});

	test('tokenCount is populated from provider usage on completion', async () => {
		const provider = createMockProvider([
			[{ delta: 'done', done: true, usage: { inputTokens: 10, outputTokens: 5 } }]
		]);
		const service = disposables.add(new ForgeAgentService(
			createMockAIProviderService(provider, disposables) as never,
			createMockMcpService(disposables) as never,
			createMockConfigResolutionService(disposables) as never,
			new NullLogService()
		));

		const task = await service.spawnAgent({
			name: 'Agent', systemPrompt: 'test', taskDescription: 'test',
			providerName: 'mock', model: 'mock-model'
		});
		await new Promise(resolve => setTimeout(resolve, 100));

		const completed = service.getAgent(task.id);
		assert.strictEqual(completed?.status, ForgeAgentStatus.Completed);
		assert.ok(completed?.tokenCount);
		assert.strictEqual(completed!.tokenCount!.input, 10);
		assert.strictEqual(completed!.tokenCount!.output, 5);
	});

	test('provider not found sets Error status with provider name', async () => {
		// AI provider service that returns undefined for any provider name.
		const noProviderService = {
			_serviceBrand: undefined as undefined,
			getProvider: (_name: string) => undefined,
			listProviders: () => [],
			has: () => false,
			registerProvider: () => { },
			unregisterProvider: () => { },
			getDefaultProviderName: () => undefined as string | undefined,
			setDefaultProviderName: () => { },
			onDidChangeProviders: disposables.add(new Emitter<string[]>()).event
		};
		const service = disposables.add(new ForgeAgentService(
			noProviderService as never,
			createMockMcpService(disposables) as never,
			createMockConfigResolutionService(disposables) as never,
			new NullLogService()
		));

		const task = await service.spawnAgent({
			name: 'Bad Agent', systemPrompt: 'test', taskDescription: 'test',
			providerName: 'nonexistent-provider', model: 'mock-model'
		});
		await new Promise(resolve => setTimeout(resolve, 100));

		const completed = service.getAgent(task.id);
		assert.ok(completed);
		assert.strictEqual(completed!.status, ForgeAgentStatus.Error);
		assert.ok(completed!.error?.includes('nonexistent-provider'));
	});

	test('spawnAgent rejects when agent name is disabled', async () => {
		const disabledConfigService = {
			_serviceBrand: undefined as undefined,
			onDidChangeResolved: disposables.add(new Emitter<ResolvedConfig>()).event,
			resolve: async () => ({ mcpServers: [], agents: [], skills: [], disabled: { mcpServers: [], agents: ['Restricted Agent'] } }),
			getCached: () => ({ mcpServers: [], agents: [], skills: [], disabled: { mcpServers: [], agents: ['Restricted Agent'] } }),
			setMcpServerDisabled: async () => { },
			setAgentDisabled: async () => { }
		};
		const service = disposables.add(new ForgeAgentService(
			createMockAIProviderService(createMockProvider([]), disposables) as never,
			createMockMcpService(disposables) as never,
			disabledConfigService as never,
			new NullLogService()
		));

		await assert.rejects(
			() => service.spawnAgent({
				name: 'Restricted Agent', systemPrompt: 'test', taskDescription: 'test',
				providerName: 'mock', model: 'mock-model'
			}),
			/disabled/
		);
	});

	test('tool not found sets Error status with tool name', async () => {
		const provider = createMockProvider([
			[
				{ delta: '', done: false, toolUse: { id: 'c1', name: 'nonexistent_tool', input: {} } },
				{ delta: '', done: true }
			],
		]);
		const mcpService = {
			...createMockMcpService(disposables),
			callTool: async (name: string) => { throw new Error(`Tool "${name}" not found`); },
		};
		const service = disposables.add(new ForgeAgentService(
			createMockAIProviderService(provider, disposables) as never,
			mcpService as never,
			createMockConfigResolutionService(disposables) as never,
			new NullLogService()
		));

		const task = await service.spawnAgent({
			name: 'Tool Agent', systemPrompt: 'test', taskDescription: 'test',
			providerName: 'mock', model: 'mock-model'
		});
		await new Promise(resolve => setTimeout(resolve, 200));

		const completed = service.getAgent(task.id);
		assert.ok(completed);
		assert.strictEqual(completed!.status, ForgeAgentStatus.Error);
		assert.ok(completed!.error?.includes('nonexistent_tool'));
	});

	test('provider streaming error sets Error status', async () => {
		const throwingProvider = {
			name: 'mock',
			availableModels: ['mock-model'],
			async *stream(): AsyncGenerator<AIStreamChunk> {
				throw new Error('Stream connection failed');
			},
			async complete() { return { content: '', model: '', inputTokens: 0, outputTokens: 0 }; },
			async validateCredentials() { return { valid: true }; }
		};
		const service = disposables.add(new ForgeAgentService(
			createMockAIProviderService(throwingProvider, disposables) as never,
			createMockMcpService(disposables) as never,
			createMockConfigResolutionService(disposables) as never,
			new NullLogService()
		));

		const task = await service.spawnAgent({
			name: 'Throw Agent', systemPrompt: 'test', taskDescription: 'test',
			providerName: 'mock', model: 'mock-model'
		});
		await new Promise(resolve => setTimeout(resolve, 100));

		const completed = service.getAgent(task.id);
		assert.ok(completed);
		assert.strictEqual(completed!.status, ForgeAgentStatus.Error);
		assert.ok(completed!.error?.includes('Stream connection failed'));
	});

	test('MCP tool call error sets step to error and agent to Error', async () => {
		const provider = createMockProvider([
			[
				{ delta: '', done: false, toolUse: { id: 'c1', name: 'read_file', input: { path: '/a' } } },
				{ delta: '', done: true }
			],
		]);
		const failingMcpService = {
			...createMockMcpService(disposables),
			callTool: async () => { throw new Error('EACCES: permission denied'); },
		};
		const service = disposables.add(new ForgeAgentService(
			createMockAIProviderService(provider, disposables) as never,
			failingMcpService as never,
			createMockConfigResolutionService(disposables) as never,
			new NullLogService()
		));

		const task = await service.spawnAgent({
			name: 'Fail Agent', systemPrompt: 'test', taskDescription: 'test',
			providerName: 'mock', model: 'mock-model'
		});
		await new Promise(resolve => setTimeout(resolve, 200));

		const completed = service.getAgent(task.id);
		assert.ok(completed);
		assert.strictEqual(completed!.status, ForgeAgentStatus.Error);
		assert.strictEqual(completed!.steps.length, 1);
		assert.strictEqual(completed!.steps[0].status, 'error');
		assert.ok(completed!.steps[0].error?.includes('EACCES'));
	});

	test('cancelAgent on a running agent sets Error status with Cancelled message', async () => {
		// Provider that pauses so we can cancel before it finishes
		let resolveStream: (() => void) | undefined;
		const pausingProvider = {
			name: 'mock',
			availableModels: ['mock-model'],
			async *stream() {
				yield { delta: 'working...', done: false };
				await new Promise<void>(r => { resolveStream = r; });
				yield { delta: ' done', done: true };
			},
			async complete() { return { content: '', model: '', inputTokens: 0, outputTokens: 0 }; },
			async validateCredentials() { return { valid: true }; }
		};
		const service = disposables.add(new ForgeAgentService(
			createMockAIProviderService(pausingProvider, disposables) as never,
			createMockMcpService(disposables) as never,
			createMockConfigResolutionService(disposables) as never,
			new NullLogService()
		));

		const task = await service.spawnAgent({
			name: 'Cancel Agent', systemPrompt: 'test', taskDescription: 'test',
			providerName: 'mock', model: 'mock-model'
		});

		// Cancel while stream is paused
		service.cancelAgent(task.id);
		resolveStream?.(); // unblock the stream
		await new Promise(resolve => setTimeout(resolve, 100));

		const cancelled = service.getAgent(task.id);
		assert.ok(cancelled);
		assert.strictEqual(cancelled!.status, ForgeAgentStatus.Error);
		assert.ok(cancelled!.error?.includes('Cancelled'));
	});

	test('cancelAgent during tool call prevents next turn', async () => {
		const provider = createMockProvider([
			[
				{ delta: '', done: false, toolUse: { id: 'c1', name: 'read_file', input: { path: '/a' } } },
				{ delta: '', done: true }
			],
			// Turn 2: should never be reached
			[{ delta: 'should not run', done: true }],
		]);

		// eslint-disable-next-line prefer-const
		let agentService!: ForgeAgentService;
		// eslint-disable-next-line prefer-const
		let capturedId: string | undefined;

		const cancellingMcpService = {
			...createMockMcpService(disposables),
			callTool: async (name: string, input: Record<string, unknown>) => {
				if (capturedId) {
					agentService.cancelAgent(capturedId);
				}
				return { callId: `${name}-1`, content: `result:${JSON.stringify(input)}`, isError: false, completedAt: Date.now() };
			},
		};

		agentService = disposables.add(new ForgeAgentService(
			createMockAIProviderService(provider, disposables) as never,
			cancellingMcpService as never,
			createMockConfigResolutionService(disposables) as never,
			new NullLogService()
		));

		const task = await agentService.spawnAgent({
			name: 'Cancel Mid Tool', systemPrompt: 'test', taskDescription: 'test',
			providerName: 'mock', model: 'mock-model'
		});
		capturedId = task.id;

		await new Promise(resolve => setTimeout(resolve, 300));

		const finished = agentService.getAgent(task.id);
		assert.ok(finished);
		// Cancelled during/after callTool — status is Error (set by cancelAgent)
		assert.strictEqual(finished!.status, ForgeAgentStatus.Error);
		assert.ok(finished!.error?.includes('Cancelled'));
		// Second turn was not executed
		assert.strictEqual(finished!.steps.length, 1);
	});

	test('cancelAgent on completed agent is a no-op', async () => {
		const provider = createMockProvider([[{ delta: 'done', done: true }]]);
		const service = disposables.add(new ForgeAgentService(
			createMockAIProviderService(provider, disposables) as never,
			createMockMcpService(disposables) as never,
			createMockConfigResolutionService(disposables) as never,
			new NullLogService()
		));

		const task = await service.spawnAgent({
			name: 'Agent', systemPrompt: 'test', taskDescription: 'test',
			providerName: 'mock', model: 'mock-model'
		});
		await new Promise(resolve => setTimeout(resolve, 100));

		// Cancel after already completed — should not throw or change status
		assert.doesNotThrow(() => service.cancelAgent(task.id));
		assert.strictEqual(service.getAgent(task.id)!.status, ForgeAgentStatus.Completed);
	});

	test('cancelAgent on unknown ID is a no-op', () => {
		const provider = createMockProvider([]);
		const service = disposables.add(new ForgeAgentService(
			createMockAIProviderService(provider, disposables) as never,
			createMockMcpService(disposables) as never,
			createMockConfigResolutionService(disposables) as never,
			new NullLogService()
		));
		assert.doesNotThrow(() => service.cancelAgent('agent_nonexistent_99999'));
	});

	test('getAgent returns undefined for unknown ID', async () => {
		const provider = createMockProvider([]);
		const service = disposables.add(new ForgeAgentService(
			createMockAIProviderService(provider, disposables) as never,
			createMockMcpService(disposables) as never,
			createMockConfigResolutionService(disposables) as never,
			new NullLogService()
		));
		assert.strictEqual(service.getAgent('agent_totally_unknown_id'), undefined);
	});

	test('onDidChangeAgent fires on Running and Completed transitions', async () => {
		const provider = createMockProvider([[{ delta: 'done', done: true }]]);
		const service = disposables.add(new ForgeAgentService(
			createMockAIProviderService(provider, disposables) as never,
			createMockMcpService(disposables) as never,
			createMockConfigResolutionService(disposables) as never,
			new NullLogService()
		));

		const firedStatuses: ForgeAgentStatus[] = [];
		disposables.add(service.onDidChangeAgent(t => { firedStatuses.push(t.status); }));

		await service.spawnAgent({
			name: 'Event Agent', systemPrompt: 'test', taskDescription: 'test',
			providerName: 'mock', model: 'mock-model'
		});
		await new Promise(resolve => setTimeout(resolve, 100));

		assert.ok(firedStatuses.includes(ForgeAgentStatus.Running));
		assert.ok(firedStatuses.includes(ForgeAgentStatus.Completed));
	});

	test('onDidAgentStep fires when a tool step completes', async () => {
		const provider = createMockProvider([
			[
				{ delta: '', done: false, toolUse: { id: 'step_1', name: 'read_file', input: { path: '/a' } } },
				{ delta: '', done: true }
			],
			[{ delta: 'done', done: true }]
		]);
		const service = disposables.add(new ForgeAgentService(
			createMockAIProviderService(provider, disposables) as never,
			createMockMcpService(disposables) as never,
			createMockConfigResolutionService(disposables) as never,
			new NullLogService()
		));

		const stepEvents: Array<{ agentId: string; toolName: string }> = [];
		disposables.add(service.onDidAgentStep(({ agentId, step }) => {
			stepEvents.push({ agentId, toolName: step.toolName });
		}));

		await service.spawnAgent({
			name: 'Step Agent', systemPrompt: 'test', taskDescription: 'test',
			providerName: 'mock', model: 'mock-model'
		});
		await new Promise(resolve => setTimeout(resolve, 200));

		const completed = stepEvents.filter(e => e.toolName === 'read_file');
		assert.ok(completed.length >= 1);
	});

	test('tool result is injected as tool_result message with correct toolCallId', async () => {
		const capturedMessages: Array<{ role: string; toolCallId?: string }> = [];
		let callCount = 0;

		// Build the provider service directly to avoid type conflicts with the stream parameter.
		const recordingProvider = {
			name: 'mock',
			availableModels: ['mock-model'],
			stream(req: { messages: Array<{ role: string; toolCallId?: string }> }): AsyncIterable<AIStreamChunk> {
				capturedMessages.push(...req.messages);
				const turn = callCount++;
				return (async function* () {
					if (turn === 0) {
						yield { delta: '', done: false, toolUse: { id: 'tc_abc', name: 'read_file', input: { path: '/x' } } };
						yield { delta: '', done: true };
					} else {
						yield { delta: 'done', done: true };
					}
				})();
			},
			async complete() { return { content: '', model: '', inputTokens: 0, outputTokens: 0 }; },
			async validateCredentials() { return { valid: true }; }
		};
		const recordingAIService = {
			_serviceBrand: undefined as undefined,
			getProvider: () => recordingProvider,
			listProviders: () => ['mock'],
			has: () => true,
			registerProvider: () => { },
			unregisterProvider: () => { },
			getDefaultProviderName: () => 'mock' as string | undefined,
			setDefaultProviderName: () => { },
			onDidChangeProviders: disposables.add(new Emitter<string[]>()).event
		};

		const service = disposables.add(new ForgeAgentService(
			recordingAIService as never,
			createMockMcpService(disposables) as never,
			createMockConfigResolutionService(disposables) as never,
			new NullLogService()
		));

		await service.spawnAgent({
			name: 'History Agent', systemPrompt: 'test', taskDescription: 'Read /x',
			providerName: 'mock', model: 'mock-model'
		});
		await new Promise(resolve => setTimeout(resolve, 300));

		const toolResultMsg = capturedMessages.find(m => m.role === 'tool_result');
		assert.ok(toolResultMsg, 'Expected a tool_result message in history');
		assert.strictEqual(toolResultMsg!.toolCallId, 'tc_abc');
	});

	test('spawnAgentFromDefinition uses definition system prompt and options', async () => {
		const provider = createMockProvider([[{ delta: 'done', done: true }]]);

		const agentDef = {
			name: 'code-reviewer',
			description: 'Reviews code',
			systemPrompt: 'You are a code reviewer. Be thorough.',
			provider: 'mock',
			model: 'mock-model',
			maxTurns: 5,
		};

		const configWithAgent = {
			_serviceBrand: undefined as undefined,
			onDidChangeResolved: disposables.add(new Emitter<ResolvedConfig>()).event,
			resolve: async () => ({ mcpServers: [], agents: [agentDef], skills: [], disabled: { mcpServers: [], agents: [] } }),
			getCached: () => ({ mcpServers: [], agents: [agentDef], skills: [], disabled: { mcpServers: [], agents: [] } }),
			setMcpServerDisabled: async () => { },
			setAgentDisabled: async () => { }
		};

		const service = disposables.add(new ForgeAgentService(
			createMockAIProviderService(provider, disposables) as never,
			createMockMcpService(disposables) as never,
			configWithAgent as never,
			new NullLogService()
		));

		const task = await service.spawnAgentFromDefinition('code-reviewer', 'Review this PR');
		assert.strictEqual(task.name, 'code-reviewer');
		assert.strictEqual(task.systemPrompt, 'You are a code reviewer. Be thorough.');
		assert.strictEqual(task.taskDescription, 'Review this PR');
		assert.strictEqual(task.maxTurns, 5);
	});

	test('spawnAgentFromDefinition with unknown name throws', async () => {
		const provider = createMockProvider([]);
		const service = disposables.add(new ForgeAgentService(
			createMockAIProviderService(provider, disposables) as never,
			createMockMcpService(disposables) as never,
			createMockConfigResolutionService(disposables) as never,
			new NullLogService()
		));

		await assert.rejects(
			() => service.spawnAgentFromDefinition('nonexistent-agent', 'task'),
			/Agent definition "nonexistent-agent" not found/,
		);
	});

	test('getAvailableDefinitions returns agents from config', async () => {
		const agentDef = {
			name: 'planner',
			description: 'Plans things',
			systemPrompt: 'You plan.',
		};
		const configWithAgent = {
			_serviceBrand: undefined as undefined,
			onDidChangeResolved: disposables.add(new Emitter<ResolvedConfig>()).event,
			resolve: async () => ({ mcpServers: [], agents: [agentDef], skills: [], disabled: { mcpServers: [], agents: [] } }),
			getCached: () => ({ mcpServers: [], agents: [agentDef], skills: [], disabled: { mcpServers: [], agents: [] } }),
			setMcpServerDisabled: async () => { },
			setAgentDisabled: async () => { }
		};
		const provider = createMockProvider([]);
		const service = disposables.add(new ForgeAgentService(
			createMockAIProviderService(provider, disposables) as never,
			createMockMcpService(disposables) as never,
			configWithAgent as never,
			new NullLogService()
		));

		const defs = service.getAvailableDefinitions();
		assert.strictEqual(defs.length, 1);
		assert.strictEqual(defs[0].name, 'planner');
	});

	test('getAllAgents and getRunningAgents', async () => {
		const provider = createMockProvider([
			[{ delta: 'done', done: true }]
		]);

		const service = disposables.add(new ForgeAgentService(
			createMockAIProviderService(provider, disposables) as never,
			createMockMcpService(disposables) as never,
			createMockConfigResolutionService(disposables) as never,
			new NullLogService()
		));

		await service.spawnAgent({
			name: 'Agent',
			systemPrompt: 'test',
			taskDescription: 'test',
			providerName: 'mock',
			model: 'mock-model'
		});

		assert.ok(service.getAllAgents().length >= 1);

		await new Promise(resolve => setTimeout(resolve, 100));
		assert.strictEqual(service.getRunningAgents().length, 0);
	});

	test('agent executes 3 sequential tool calls before text completion', async () => {
		const provider = createMockProvider([
			[{ delta: '', done: false, toolUse: { id: 'c1', name: 'read_file', input: { path: '/a' } } }, { delta: '', done: true }],
			[{ delta: '', done: false, toolUse: { id: 'c2', name: 'read_file', input: { path: '/b' } } }, { delta: '', done: true }],
			[{ delta: '', done: false, toolUse: { id: 'c3', name: 'read_file', input: { path: '/c' } } }, { delta: '', done: true }],
			[{ delta: 'All done.', done: true }],
		]);

		const service = disposables.add(new ForgeAgentService(
			createMockAIProviderService(provider, disposables) as never,
			createMockMcpService(disposables) as never,
			createMockConfigResolutionService(disposables) as never,
			new NullLogService()
		));

		const task = await service.spawnAgent({
			name: 'Multi Tool Agent', systemPrompt: 'You read files.', taskDescription: 'Read three files.',
			providerName: 'mock', model: 'mock-model'
		});
		await new Promise(resolve => setTimeout(resolve, 500));

		const completed = service.getAgent(task.id);
		assert.ok(completed);
		assert.strictEqual(completed!.status, ForgeAgentStatus.Completed);
		assert.strictEqual(completed!.steps.length, 3);
		assert.strictEqual(completed!.result, 'All done.');
	});

	test('currentTurn equals maxTurns exactly at MaxTurnsReached', async () => {
		const infiniteToolCalls: AIStreamChunk[][] = Array(10).fill([
			{ delta: '', done: false, toolUse: { id: 'c', name: 'read_file', input: { path: '/a' } } },
			{ delta: '', done: true }
		]);
		const provider = createMockProvider(infiniteToolCalls);
		const service = disposables.add(new ForgeAgentService(
			createMockAIProviderService(provider, disposables) as never,
			createMockMcpService(disposables) as never,
			createMockConfigResolutionService(disposables) as never,
			new NullLogService()
		));

		const task = await service.spawnAgent({
			name: 'Turn Counter', systemPrompt: 'Loop.', taskDescription: 'Loop.',
			providerName: 'mock', model: 'mock-model', maxTurns: 4
		});
		await new Promise(resolve => setTimeout(resolve, 500));

		const completed = service.getAgent(task.id);
		assert.ok(completed);
		assert.strictEqual(completed!.status, ForgeAgentStatus.MaxTurnsReached);
		assert.strictEqual(completed!.currentTurn, 4);
	});

	test('maxTurns:2 stops the loop after exactly 2 tool calls', async () => {
		const infiniteToolCalls: AIStreamChunk[][] = Array(10).fill([
			{ delta: '', done: false, toolUse: { id: 'c', name: 'read_file', input: { path: '/x' } } },
			{ delta: '', done: true }
		]);
		const provider = createMockProvider(infiniteToolCalls);
		const service = disposables.add(new ForgeAgentService(
			createMockAIProviderService(provider, disposables) as never,
			createMockMcpService(disposables) as never,
			createMockConfigResolutionService(disposables) as never,
			new NullLogService()
		));

		const task = await service.spawnAgent({
			name: 'Short Loop', systemPrompt: 'Loop.', taskDescription: 'Loop.',
			providerName: 'mock', model: 'mock-model', maxTurns: 2
		});
		await new Promise(resolve => setTimeout(resolve, 300));

		const completed = service.getAgent(task.id);
		assert.ok(completed);
		assert.strictEqual(completed!.status, ForgeAgentStatus.MaxTurnsReached);
		assert.strictEqual(completed!.currentTurn, 2);
		assert.strictEqual(completed!.steps.length, 2);
	});

	test('getRunningAgents returns empty after agent is cancelled', async () => {
		let resolveStream: (() => void) | undefined;
		const pausingProvider = {
			name: 'mock', availableModels: ['mock-model'],
			async *stream() {
				yield { delta: 'working...', done: false };
				await new Promise<void>(r => { resolveStream = r; });
				yield { delta: ' done', done: true };
			},
			async complete() { return { content: '', model: '', inputTokens: 0, outputTokens: 0 }; },
			async validateCredentials() { return { valid: true }; }
		};
		const service = disposables.add(new ForgeAgentService(
			createMockAIProviderService(pausingProvider, disposables) as never,
			createMockMcpService(disposables) as never,
			createMockConfigResolutionService(disposables) as never,
			new NullLogService()
		));

		const task = await service.spawnAgent({
			name: 'Cancel Running', systemPrompt: 'test', taskDescription: 'test',
			providerName: 'mock', model: 'mock-model'
		});

		assert.ok(service.getRunningAgents().some(a => a.id === task.id));

		service.cancelAgent(task.id);
		resolveStream?.();
		await new Promise(resolve => setTimeout(resolve, 100));

		assert.strictEqual(service.getRunningAgents().length, 0);
	});

	test('message history contains system + user + assistant + tool_result across two turns', async () => {
		const allCallMessages: Array<Array<{ role: string }>> = [];
		let callCount = 0;

		const recordingProvider = {
			name: 'mock', availableModels: ['mock-model'],
			stream(req: { messages: Array<{ role: string }> }): AsyncIterable<AIStreamChunk> {
				allCallMessages.push([...req.messages]);
				const turn = callCount++;
				return (async function* () {
					if (turn < 2) {
						yield { delta: '', done: false, toolUse: { id: `c${turn}`, name: 'read_file', input: { path: `/f${turn}` } } };
						yield { delta: '', done: true };
					} else {
						yield { delta: 'done', done: true };
					}
				})();
			},
			async complete() { return { content: '', model: '', inputTokens: 0, outputTokens: 0 }; },
			async validateCredentials() { return { valid: true }; }
		};
		const recordingAIService = {
			_serviceBrand: undefined as undefined,
			getProvider: () => recordingProvider,
			listProviders: () => ['mock'], has: () => true,
			registerProvider: () => { }, unregisterProvider: () => { },
			getDefaultProviderName: () => 'mock' as string | undefined,
			setDefaultProviderName: () => { },
			onDidChangeProviders: disposables.add(new Emitter<string[]>()).event
		};

		const service = disposables.add(new ForgeAgentService(
			recordingAIService as never,
			createMockMcpService(disposables) as never,
			createMockConfigResolutionService(disposables) as never,
			new NullLogService()
		));

		await service.spawnAgent({
			name: 'History Order Agent', systemPrompt: 'You are a reader.', taskDescription: 'Read two files.',
			providerName: 'mock', model: 'mock-model'
		});
		await new Promise(resolve => setTimeout(resolve, 400));

		// Turn 3 call receives: system, user, assistant, tool_result, assistant, tool_result
		const finalMessages = allCallMessages[2];
		assert.ok(finalMessages, 'Expected 3 provider calls');
		assert.strictEqual(finalMessages[0].role, 'system');
		assert.strictEqual(finalMessages[1].role, 'user');
		assert.strictEqual(finalMessages[2].role, 'assistant');
		assert.strictEqual(finalMessages[3].role, 'tool_result');
		assert.strictEqual(finalMessages[4].role, 'assistant');
		assert.strictEqual(finalMessages[5].role, 'tool_result');
	});

	test('system prompt is included in every provider call', async () => {
		const capturedSystemPrompts: string[] = [];
		let callCount = 0;

		const recordingProvider = {
			name: 'mock', availableModels: ['mock-model'],
			stream(req: { messages: Array<{ role: string; content: string }> }): AsyncIterable<AIStreamChunk> {
				const sys = req.messages.find(m => m.role === 'system');
				if (sys) { capturedSystemPrompts.push(sys.content); }
				const turn = callCount++;
				return (async function* () {
					if (turn === 0) {
						yield { delta: '', done: false, toolUse: { id: 'c1', name: 'read_file', input: { path: '/a' } } };
						yield { delta: '', done: true };
					} else {
						yield { delta: 'done', done: true };
					}
				})();
			},
			async complete() { return { content: '', model: '', inputTokens: 0, outputTokens: 0 }; },
			async validateCredentials() { return { valid: true }; }
		};
		const recordingAIService = {
			_serviceBrand: undefined as undefined,
			getProvider: () => recordingProvider,
			listProviders: () => ['mock'], has: () => true,
			registerProvider: () => { }, unregisterProvider: () => { },
			getDefaultProviderName: () => 'mock' as string | undefined,
			setDefaultProviderName: () => { },
			onDidChangeProviders: disposables.add(new Emitter<string[]>()).event
		};

		const service = disposables.add(new ForgeAgentService(
			recordingAIService as never,
			createMockMcpService(disposables) as never,
			createMockConfigResolutionService(disposables) as never,
			new NullLogService()
		));

		await service.spawnAgent({
			name: 'System Prompt Agent', systemPrompt: 'You are a unique assistant.',
			taskDescription: 'Read /a.', providerName: 'mock', model: 'mock-model'
		});
		await new Promise(resolve => setTimeout(resolve, 300));

		assert.strictEqual(capturedSystemPrompts.length, 2);
		assert.ok(capturedSystemPrompts.every(p => p === 'You are a unique assistant.'));
	});

	test('onDidChangeAgent fires on Error status when provider not found', async () => {
		const noProviderService = {
			_serviceBrand: undefined as undefined,
			getProvider: (_name: string) => undefined,
			listProviders: () => [], has: () => false,
			registerProvider: () => { }, unregisterProvider: () => { },
			getDefaultProviderName: () => undefined as string | undefined,
			setDefaultProviderName: () => { },
			onDidChangeProviders: disposables.add(new Emitter<string[]>()).event
		};
		const service = disposables.add(new ForgeAgentService(
			noProviderService as never,
			createMockMcpService(disposables) as never,
			createMockConfigResolutionService(disposables) as never,
			new NullLogService()
		));

		const firedStatuses: ForgeAgentStatus[] = [];
		disposables.add(service.onDidChangeAgent(t => firedStatuses.push(t.status)));

		await service.spawnAgent({
			name: 'Error Agent', systemPrompt: 'test', taskDescription: 'test',
			providerName: 'nonexistent', model: 'mock-model'
		});
		await new Promise(resolve => setTimeout(resolve, 100));

		assert.ok(firedStatuses.includes(ForgeAgentStatus.Error));
	});

	test('disposed onDidChangeAgent listener is not called after disposal', async () => {
		const provider = createMockProvider([[{ delta: 'done', done: true }]]);
		const service = disposables.add(new ForgeAgentService(
			createMockAIProviderService(provider, disposables) as never,
			createMockMcpService(disposables) as never,
			createMockConfigResolutionService(disposables) as never,
			new NullLogService()
		));

		let callCount = 0;
		const sub = service.onDidChangeAgent(() => { callCount++; });
		sub.dispose();

		await service.spawnAgent({
			name: 'Disposed Listener Agent', systemPrompt: 'test', taskDescription: 'test',
			providerName: 'mock', model: 'mock-model'
		});
		await new Promise(resolve => setTimeout(resolve, 100));

		assert.strictEqual(callCount, 0);
	});

	test('spawnAgentFromDefinition sets allowedTools from definition tools array', async () => {
		const provider = createMockProvider([[{ delta: 'done', done: true }]]);
		const agentDef = {
			name: 'selective-agent',
			description: 'Uses only specific tools',
			systemPrompt: 'You are selective.',
			provider: 'mock',
			model: 'mock-model',
			tools: ['read_file', 'write_file'],
		};
		const configWithAgent = {
			_serviceBrand: undefined as undefined,
			onDidChangeResolved: disposables.add(new Emitter<ResolvedConfig>()).event,
			resolve: async () => ({ mcpServers: [], agents: [agentDef], skills: [], disabled: { mcpServers: [], agents: [] } }),
			getCached: () => ({ mcpServers: [], agents: [agentDef], skills: [], disabled: { mcpServers: [], agents: [] } }),
			setMcpServerDisabled: async () => { },
			setAgentDisabled: async () => { }
		};

		const service = disposables.add(new ForgeAgentService(
			createMockAIProviderService(provider, disposables) as never,
			createMockMcpService(disposables) as never,
			configWithAgent as never,
			new NullLogService()
		));

		const task = await service.spawnAgentFromDefinition('selective-agent', 'Do something');
		assert.deepStrictEqual(task.allowedTools, ['read_file', 'write_file']);
	});

	test('allowedTools filters tools passed to the provider', async () => {
		const capturedTools: string[][] = [];

		const recordingProvider = {
			name: 'mock', availableModels: ['mock-model'],
			stream(req: AICompletionRequest): AsyncIterable<AIStreamChunk> {
				capturedTools.push((req.tools ?? []).map(t => t.name));
				return (async function* () { yield { delta: 'done', done: true }; })();
			},
			async complete() { return { content: '', model: '', inputTokens: 0, outputTokens: 0 }; },
			async validateCredentials() { return { valid: true }; }
		};

		const multiToolMcpService = {
			_serviceBrand: undefined as undefined,
			onDidChangeTools: disposables.add(new Emitter<void>()).event,
			onDidChangeServerStatus: disposables.add(new Emitter<ForgeMcpServerStatusEntry>()).event,
			listTools: async (): Promise<AIToolDefinition[]> => [
				{ name: 'read_file', description: 'Read', inputSchema: {} },
				{ name: 'write_file', description: 'Write', inputSchema: {} },
				{ name: 'delete_file', description: 'Delete', inputSchema: {} },
			],
			callTool: async () => ({ callId: 'c1', content: 'ok', isError: false, completedAt: Date.now() }),
			getServerStatuses: () => [],
			isServerDisabled: () => false,
			toggleServerDisabled: async () => { }
		};

		const recordingAIService = {
			_serviceBrand: undefined as undefined,
			getProvider: () => recordingProvider,
			listProviders: () => ['mock'], has: () => true,
			registerProvider: () => { }, unregisterProvider: () => { },
			getDefaultProviderName: () => 'mock' as string | undefined,
			setDefaultProviderName: () => { },
			onDidChangeProviders: disposables.add(new Emitter<string[]>()).event
		};

		const service = disposables.add(new ForgeAgentService(
			recordingAIService as never,
			multiToolMcpService as never,
			createMockConfigResolutionService(disposables) as never,
			new NullLogService()
		));

		await service.spawnAgent({
			name: 'Filtered Agent', systemPrompt: 'test', taskDescription: 'test',
			providerName: 'mock', model: 'mock-model',
			allowedTools: ['read_file', 'write_file'],
		});
		await new Promise(resolve => setTimeout(resolve, 100));

		assert.strictEqual(capturedTools.length, 1);
		assert.deepStrictEqual(capturedTools[0].sort(), ['read_file', 'write_file']);
	});

	test('getAvailableDefinitions excludes disabled agents', () => {
		const plannerDef = { name: 'planner', description: 'Plans things', systemPrompt: 'You plan.' };
		const reviewerDef = { name: 'reviewer', description: 'Reviews things', systemPrompt: 'You review.' };
		const configWithDisabled = {
			_serviceBrand: undefined as undefined,
			onDidChangeResolved: disposables.add(new Emitter<ResolvedConfig>()).event,
			resolve: async () => ({ mcpServers: [], agents: [plannerDef, reviewerDef], skills: [], disabled: { mcpServers: [], agents: ['reviewer'] } }),
			getCached: () => ({ mcpServers: [], agents: [plannerDef, reviewerDef], skills: [], disabled: { mcpServers: [], agents: ['reviewer'] } }),
			setMcpServerDisabled: async () => { },
			setAgentDisabled: async () => { }
		};
		const provider = createMockProvider([]);
		const service = disposables.add(new ForgeAgentService(
			createMockAIProviderService(provider, disposables) as never,
			createMockMcpService(disposables) as never,
			configWithDisabled as never,
			new NullLogService()
		));

		const defs = service.getAvailableDefinitions();
		assert.strictEqual(defs.length, 1);
		assert.strictEqual(defs[0].name, 'planner');
	});

	test('two concurrent agents maintain independent state', async () => {
		let agentAResolve: (() => void) | undefined;
		let agentBResolve: (() => void) | undefined;

		const makeGatedProvider = (result: string, gate: Promise<void>) => ({
			name: 'mock', availableModels: ['mock-model'],
			async *stream() {
				await gate;
				yield { delta: result, done: true };
			},
			async complete() { return { content: '', model: '', inputTokens: 0, outputTokens: 0 }; },
			async validateCredentials() { return { valid: true }; }
		});

		const gateA = new Promise<void>(r => { agentAResolve = r; });
		const gateB = new Promise<void>(r => { agentBResolve = r; });

		const serviceA = disposables.add(new ForgeAgentService(
			createMockAIProviderService(makeGatedProvider('result-A', gateA), disposables) as never,
			createMockMcpService(disposables) as never,
			createMockConfigResolutionService(disposables) as never,
			new NullLogService()
		));
		const serviceB = disposables.add(new ForgeAgentService(
			createMockAIProviderService(makeGatedProvider('result-B', gateB), disposables) as never,
			createMockMcpService(disposables) as never,
			createMockConfigResolutionService(disposables) as never,
			new NullLogService()
		));

		const taskAPromise = serviceA.spawnAgent({ name: 'Agent-A', systemPrompt: 'A', taskDescription: 'task-A', providerName: 'mock', model: 'mock-model' });
		const taskBPromise = serviceB.spawnAgent({ name: 'Agent-B', systemPrompt: 'B', taskDescription: 'task-B', providerName: 'mock', model: 'mock-model' });

		const [taskA, taskB] = await Promise.all([taskAPromise, taskBPromise]);
		assert.notStrictEqual(taskA.id, taskB.id);

		agentBResolve!();
		agentAResolve!();
		await new Promise(resolve => setTimeout(resolve, 100));

		assert.strictEqual(taskA.status, ForgeAgentStatus.Completed);
		assert.strictEqual(taskB.status, ForgeAgentStatus.Completed);
		assert.strictEqual(taskA.result, 'result-A');
		assert.strictEqual(taskB.result, 'result-B');
		assert.strictEqual(taskA.steps.length, 0);
		assert.strictEqual(taskB.steps.length, 0);
	});
});
