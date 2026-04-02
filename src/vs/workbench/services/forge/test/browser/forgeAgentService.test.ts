import assert from 'assert';
import { Emitter } from '../../../../../base/common/event.js';
import { DisposableStore } from '../../../../../base/common/lifecycle.js';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import { ForgeAgentService } from '../../browser/forgeAgentService.js';
import { ForgeAgentStatus } from '../../common/forgeAgentTypes.js';
import { NullLogService } from '../../../../../platform/log/common/log.js';
import { AIStreamChunk, AIToolDefinition } from '../../../../../platform/ai/common/aiProvider.js';
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

	function createMockAIProviderService(provider: ReturnType<typeof createMockProvider>) {
		return {
			_serviceBrand: undefined as undefined,
			getProvider: () => provider,
			listProviders: () => ['mock'],
			has: () => true,
			registerProvider: () => { },
			unregisterProvider: () => { },
			getDefaultProviderName: () => 'mock' as string | undefined,
			setDefaultProviderName: () => { },
			onDidChangeProviders: new Emitter<string[]>().event
		};
	}

	function createMockMcpService() {
		return {
			_serviceBrand: undefined as undefined,
			onDidChangeTools: new Emitter<void>().event,
			onDidChangeServerStatus: new Emitter<ForgeMcpServerStatusEntry>().event,
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

	function createMockConfigResolutionService() {
		return {
			_serviceBrand: undefined as undefined,
			onDidChangeResolved: new Emitter<ResolvedConfig>().event,
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
			createMockAIProviderService(provider) as never,
			createMockMcpService() as never,
			createMockConfigResolutionService() as never,
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
			createMockAIProviderService(provider) as never,
			createMockMcpService() as never,
			createMockConfigResolutionService() as never,
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
			createMockAIProviderService(provider) as never,
			createMockMcpService() as never,
			createMockConfigResolutionService() as never,
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

	test('getAllAgents and getRunningAgents', async () => {
		const provider = createMockProvider([
			[{ delta: 'done', done: true }]
		]);

		const service = disposables.add(new ForgeAgentService(
			createMockAIProviderService(provider) as never,
			createMockMcpService() as never,
			createMockConfigResolutionService() as never,
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
});
