import assert from 'assert';
import { Emitter } from '../../../../../base/common/event.js';
import { DisposableStore } from '../../../../../base/common/lifecycle.js';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import { NullLogService } from '../../../../../platform/log/common/log.js';
import { ForgeMcpService } from '../../browser/forgeMcpService.js';
import { ForgeMcpServerStatus } from '../../common/forgeMcpTypes.js';
import { ResolvedConfig } from '../../common/forgeConfigResolutionTypes.js';

/** McpConnectionState.Kind values matching vs/workbench/contrib/mcp/common/mcpTypes.ts */
const enum McpConnectionState {
	Stopped = 0,
	Starting = 1,
	Running = 2,
	Error = 3,
}

suite('ForgeMcpService', () => {
	let disposables: DisposableStore;

	setup(() => {
		disposables = new DisposableStore();
	});

	teardown(() => {
		disposables.dispose();
	});

	ensureNoDisposablesAreLeakedInTestSuite();

	function createMockMcpServer(name: string, connectionKind: McpConnectionState, tools: { name: string; description: string; inputSchema: Record<string, unknown> }[]) {
		const state = { state: connectionKind as number };

		const mockTools = tools.map(t => ({
			id: t.name,
			referenceName: t.name,
			icons: {},
			definition: {
				name: t.name,
				description: t.description,
				inputSchema: { type: 'object', properties: t.inputSchema },
			},
			visibility: 3,
			call: async (input: Record<string, unknown>) => ({
				content: [{ type: 'text' as const, text: `result:${JSON.stringify(input)}` }],
				isError: false,
			}),
			callWithProgress: async () => ({ content: [], isError: false }),
		}));

		return {
			collection: { id: 'col', label: 'col' },
			definition: { id: name, label: name },
			enablement: { get: () => 0, read: () => 0 } as never,
			connection: { get: () => undefined, read: () => undefined } as never,
			connectionState: { get: () => state, read: () => state } as never,
			serverMetadata: { get: () => undefined } as never,
			capabilities: { get: () => undefined } as never,
			tools: { get: () => mockTools, read: () => mockTools } as never,
			prompts: { get: () => [], read: () => [] } as never,
			cacheState: { get: () => 0 } as never,
			readDefinitions: () => ({ get: () => ({ server: undefined, collection: undefined }) }) as never,
			showOutput: async () => undefined,
			start: async () => state as never,
			stop: async () => undefined,
			resources: async function* () { yield []; },
			resourceTemplates: async () => [],
			dispose: () => undefined,
		};
	}

	function createMockMcpService(servers: ReturnType<typeof createMockMcpServer>[]) {
		return {
			_serviceBrand: undefined,
			servers: { get: () => servers, read: () => servers } as never,
			enablementModel: {} as never,
			resetCaches: () => undefined,
			resetTrust: () => undefined,
			lazyCollectionState: { get: () => ({ state: 2, collections: [] }) } as never,
			autostart: () => ({ get: () => ({ working: false, starting: [], serversRequiringInteraction: [] }) }) as never,
			cancelAutostart: () => undefined,
			activateCollections: async () => undefined,
		};
	}

	function createMockConfigResolution(disabledMcpServers: string[] = []) {
		const _onDidChangeResolved = disposables.add(new Emitter<ResolvedConfig>());
		const cachedConfig: ResolvedConfig = {
			mcpServers: [],
			agents: [],
			skills: [],
			disabled: { mcpServers: disabledMcpServers, agents: [] },
		};
		return {
			_serviceBrand: undefined,
			onDidChangeResolved: _onDidChangeResolved.event,
			resolve: async () => cachedConfig,
			getCached: () => cachedConfig,
			setMcpServerDisabled: async (_name: string, _disabled: boolean) => undefined,
			setAgentDisabled: async (_name: string, _disabled: boolean) => undefined,
		};
	}

	test('listTools returns empty when no servers connected', async () => {
		const mcpService = createMockMcpService([]);
		const configResolution = createMockConfigResolution();
		const service = disposables.add(new ForgeMcpService(
			mcpService as never,
			configResolution as never,
			new NullLogService(),
		));

		const tools = await service.listTools();
		assert.strictEqual(tools.length, 0);
	});

	test('listTools returns tools from running servers', async () => {
		const server = createMockMcpServer('test-server', McpConnectionState.Running, [
			{ name: 'read_file', description: 'Read a file', inputSchema: {} },
			{ name: 'write_file', description: 'Write a file', inputSchema: {} },
		]);
		const mcpService = createMockMcpService([server]);
		const configResolution = createMockConfigResolution();
		const service = disposables.add(new ForgeMcpService(
			mcpService as never,
			configResolution as never,
			new NullLogService(),
		));

		const tools = await service.listTools();
		assert.strictEqual(tools.length, 2);
		assert.strictEqual(tools[0].name, 'read_file');
		assert.strictEqual(tools[1].name, 'write_file');
	});

	test('listTools skips disabled servers', async () => {
		const server = createMockMcpServer('noisy-server', McpConnectionState.Running, [
			{ name: 'some_tool', description: 'A tool', inputSchema: {} },
		]);
		const mcpService = createMockMcpService([server]);
		const configResolution = createMockConfigResolution(['noisy-server']);
		const service = disposables.add(new ForgeMcpService(
			mcpService as never,
			configResolution as never,
			new NullLogService(),
		));

		const tools = await service.listTools();
		assert.strictEqual(tools.length, 0);
	});

	test('getServerStatuses returns empty when no servers', () => {
		const mcpService = createMockMcpService([]);
		const configResolution = createMockConfigResolution();
		const service = disposables.add(new ForgeMcpService(
			mcpService as never,
			configResolution as never,
			new NullLogService(),
		));

		const statuses = service.getServerStatuses();
		assert.deepStrictEqual(statuses, []);
	});

	test('getServerStatuses maps connection states to ForgeMcpServerStatus', () => {
		const servers = [
			createMockMcpServer('running-server', McpConnectionState.Running, [
				{ name: 'tool_a', description: '', inputSchema: {} },
			]),
			createMockMcpServer('stopped-server', McpConnectionState.Stopped, []),
			createMockMcpServer('error-server', McpConnectionState.Error, []),
		];
		const mcpService = createMockMcpService(servers);
		const configResolution = createMockConfigResolution();
		const service = disposables.add(new ForgeMcpService(
			mcpService as never,
			configResolution as never,
			new NullLogService(),
		));

		const statuses = service.getServerStatuses();
		assert.strictEqual(statuses.length, 3);

		const runningEntry = statuses.find(s => s.name === 'running-server');
		assert.ok(runningEntry);
		assert.strictEqual(runningEntry.status, ForgeMcpServerStatus.Connected);
		assert.strictEqual(runningEntry.toolCount, 1);
		assert.strictEqual(runningEntry.disabled, false);

		const stoppedEntry = statuses.find(s => s.name === 'stopped-server');
		assert.ok(stoppedEntry);
		assert.strictEqual(stoppedEntry.status, ForgeMcpServerStatus.Disconnected);

		const errorEntry = statuses.find(s => s.name === 'error-server');
		assert.ok(errorEntry);
		assert.strictEqual(errorEntry.status, ForgeMcpServerStatus.Error);
	});

	test('callTool throws when tool not found', async () => {
		const mcpService = createMockMcpService([]);
		const configResolution = createMockConfigResolution();
		const service = disposables.add(new ForgeMcpService(
			mcpService as never,
			configResolution as never,
			new NullLogService(),
		));

		await assert.rejects(
			() => service.callTool('nonexistent', {}),
			/Tool "nonexistent" not found/,
		);
	});

	test('callTool returns result record for found tool', async () => {
		const server = createMockMcpServer('fs-server', McpConnectionState.Running, [
			{ name: 'read_file', description: 'Read a file', inputSchema: {} },
		]);
		const mcpService = createMockMcpService([server]);
		const configResolution = createMockConfigResolution();
		const service = disposables.add(new ForgeMcpService(
			mcpService as never,
			configResolution as never,
			new NullLogService(),
		));

		const result = await service.callTool('read_file', { path: '/tmp/test.txt' });
		assert.strictEqual(result.isError, false);
		assert.ok(result.content.includes('result:'));
		assert.ok(result.callId.startsWith('read_file-'));
		assert.ok(result.completedAt > 0);
	});

	test('isServerDisabled reflects config resolution disabled list', () => {
		const mcpService = createMockMcpService([]);
		const configResolution = createMockConfigResolution(['noisy-server']);
		const service = disposables.add(new ForgeMcpService(
			mcpService as never,
			configResolution as never,
			new NullLogService(),
		));

		assert.strictEqual(service.isServerDisabled('noisy-server'), true);
		assert.strictEqual(service.isServerDisabled('other-server'), false);
	});

	test('isServerDisabled returns false when no cached config', () => {
		const mcpService = createMockMcpService([]);
		const _onDidChangeResolved = disposables.add(new Emitter<ResolvedConfig>());
		const configResolution = {
			_serviceBrand: undefined,
			onDidChangeResolved: _onDidChangeResolved.event,
			resolve: async () => ({ mcpServers: [], agents: [], skills: [], disabled: { mcpServers: [], agents: [] } }),
			getCached: () => undefined,
			setMcpServerDisabled: async () => undefined,
			setAgentDisabled: async () => undefined,
		};
		const service = disposables.add(new ForgeMcpService(
			mcpService as never,
			configResolution as never,
			new NullLogService(),
		));

		assert.strictEqual(service.isServerDisabled('any-server'), false);
	});

	test('onDidChangeTools fires when config resolution changes', async () => {
		const mcpService = createMockMcpService([]);
		const _onDidChangeResolved = disposables.add(new Emitter<ResolvedConfig>());
		const configResolution = {
			_serviceBrand: undefined,
			onDidChangeResolved: _onDidChangeResolved.event,
			resolve: async () => ({ mcpServers: [], agents: [], skills: [], disabled: { mcpServers: [], agents: [] } }),
			getCached: () => undefined,
			setMcpServerDisabled: async () => undefined,
			setAgentDisabled: async () => undefined,
		};
		const service = disposables.add(new ForgeMcpService(
			mcpService as never,
			configResolution as never,
			new NullLogService(),
		));

		let fired = false;
		disposables.add(service.onDidChangeTools(() => { fired = true; }));

		const dummyConfig: ResolvedConfig = { mcpServers: [], agents: [], skills: [], disabled: { mcpServers: [], agents: [] } };
		_onDidChangeResolved.fire(dummyConfig);

		assert.strictEqual(fired, true);
	});

	test('listTools skips servers in Connecting (Starting) state', async () => {
		const server = createMockMcpServer('starting-server', McpConnectionState.Starting, [
			{ name: 'some_tool', description: 'A tool', inputSchema: {} },
		]);
		const mcpService = createMockMcpService([server]);
		const configResolution = createMockConfigResolution();
		const service = disposables.add(new ForgeMcpService(
			mcpService as never,
			configResolution as never,
			new NullLogService(),
		));

		const tools = await service.listTools();
		assert.strictEqual(tools.length, 0);
	});

	test('listTools returns tools from multiple enabled running servers', async () => {
		const serverA = createMockMcpServer('server-a', McpConnectionState.Running, [
			{ name: 'tool_a', description: 'Tool A', inputSchema: {} },
		]);
		const serverB = createMockMcpServer('server-b', McpConnectionState.Running, [
			{ name: 'tool_b', description: 'Tool B', inputSchema: {} },
			{ name: 'tool_c', description: 'Tool C', inputSchema: {} },
		]);
		const mcpService = createMockMcpService([serverA, serverB]);
		const configResolution = createMockConfigResolution();
		const service = disposables.add(new ForgeMcpService(
			mcpService as never,
			configResolution as never,
			new NullLogService(),
		));

		const tools = await service.listTools();
		assert.strictEqual(tools.length, 3);
	});

	test('callTool result preserves isError: true from server', async () => {
		// Build the server directly so we can control the tool's call() return value.
		const errorTools = [{
			id: 'read_file', referenceName: 'read_file', icons: {}, visibility: 3,
			definition: { name: 'read_file', description: 'Read a file', inputSchema: {} },
			call: async (_input: Record<string, unknown>) => ({
				content: [{ type: 'text' as const, text: 'Permission denied' }],
				isError: true,
			}),
			callWithProgress: async () => ({ content: [], isError: false }),
		}];
		const baseServer = createMockMcpServer('fs-server', McpConnectionState.Running, []);
		const server = { ...baseServer, tools: { get: () => errorTools, read: () => errorTools } as never };

		const mcpService = createMockMcpService([server]);
		const configResolution = createMockConfigResolution();
		const service = disposables.add(new ForgeMcpService(
			mcpService as never,
			configResolution as never,
			new NullLogService(),
		));

		const result = await service.callTool('read_file', { path: '/etc/shadow' });
		assert.strictEqual(result.isError, true);
		assert.ok(result.content.includes('Permission denied'));
	});

	test('callTool joins multi-part content array into single string', async () => {
		const multiPartTools = [{
			id: 'read_file', referenceName: 'read_file', icons: {}, visibility: 3,
			definition: { name: 'read_file', description: 'Read a file', inputSchema: {} },
			call: async (_input: Record<string, unknown>) => ({
				content: [
					{ type: 'text' as const, text: 'Line 1' },
					{ type: 'text' as const, text: 'Line 2' },
					{ type: 'text' as const, text: 'Line 3' },
				],
				isError: false,
			}),
			callWithProgress: async () => ({ content: [], isError: false }),
		}];
		const baseServer = createMockMcpServer('fs-server', McpConnectionState.Running, []);
		const server = { ...baseServer, tools: { get: () => multiPartTools, read: () => multiPartTools } as never };

		const mcpService = createMockMcpService([server]);
		const configResolution = createMockConfigResolution();
		const service = disposables.add(new ForgeMcpService(
			mcpService as never,
			configResolution as never,
			new NullLogService(),
		));

		const result = await service.callTool('read_file', {});
		assert.ok(result.content.includes('Line 1'));
		assert.ok(result.content.includes('Line 2'));
		assert.ok(result.content.includes('Line 3'));
	});

	test('callTool on disabled server skips it and throws tool not found', async () => {
		const server = createMockMcpServer('disabled-server', McpConnectionState.Running, [
			{ name: 'secret_tool', description: 'A secret tool', inputSchema: {} },
		]);
		const mcpService = createMockMcpService([server]);
		const configResolution = createMockConfigResolution(['disabled-server']);
		const service = disposables.add(new ForgeMcpService(
			mcpService as never,
			configResolution as never,
			new NullLogService(),
		));

		await assert.rejects(
			() => service.callTool('secret_tool', {}),
			/Tool "secret_tool" not found/,
		);
	});

	test('getServerStatuses maps Starting to Connecting', () => {
		const server = createMockMcpServer('starting-server', McpConnectionState.Starting, []);
		const mcpService = createMockMcpService([server]);
		const configResolution = createMockConfigResolution();
		const service = disposables.add(new ForgeMcpService(
			mcpService as never,
			configResolution as never,
			new NullLogService(),
		));

		const statuses = service.getServerStatuses();
		assert.strictEqual(statuses.length, 1);
		assert.strictEqual(statuses[0].status, ForgeMcpServerStatus.Connecting);
	});

	test('getServerStatuses marks disabled server correctly', () => {
		const server = createMockMcpServer('noisy-server', McpConnectionState.Running, [
			{ name: 'tool', description: '', inputSchema: {} },
		]);
		const mcpService = createMockMcpService([server]);
		const configResolution = createMockConfigResolution(['noisy-server']);
		const service = disposables.add(new ForgeMcpService(
			mcpService as never,
			configResolution as never,
			new NullLogService(),
		));

		const statuses = service.getServerStatuses();
		assert.strictEqual(statuses.length, 1);
		assert.strictEqual(statuses[0].disabled, true);
	});

	test('toggleServerDisabled with disabled:false delegates re-enable', async () => {
		const mcpService = createMockMcpService([]);
		let calledWith: { name: string; disabled: boolean } | undefined;
		const _onDidChangeResolved = disposables.add(new Emitter<ResolvedConfig>());
		const configResolution = {
			_serviceBrand: undefined,
			onDidChangeResolved: _onDidChangeResolved.event,
			resolve: async () => ({ mcpServers: [], agents: [], skills: [], disabled: { mcpServers: [], agents: [] } }),
			getCached: () => undefined,
			setMcpServerDisabled: async (name: string, disabled: boolean) => { calledWith = { name, disabled }; },
			setAgentDisabled: async () => undefined,
		};
		const service = disposables.add(new ForgeMcpService(
			mcpService as never,
			configResolution as never,
			new NullLogService(),
		));

		await service.toggleServerDisabled('my-server', false);
		assert.deepStrictEqual(calledWith, { name: 'my-server', disabled: false });
	});

	test('onDidChangeServerStatus fires when config resolution changes', async () => {
		const server = createMockMcpServer('srv', McpConnectionState.Running, []);
		const mcpService = createMockMcpService([server]);
		const _onDidChangeResolved = disposables.add(new Emitter<ResolvedConfig>());
		const configResolution = {
			_serviceBrand: undefined,
			onDidChangeResolved: _onDidChangeResolved.event,
			resolve: async () => ({ mcpServers: [], agents: [], skills: [], disabled: { mcpServers: [], agents: [] } }),
			getCached: () => ({ mcpServers: [], agents: [], skills: [], disabled: { mcpServers: [], agents: [] } }),
			setMcpServerDisabled: async () => undefined,
			setAgentDisabled: async () => undefined,
		};
		const service = disposables.add(new ForgeMcpService(
			mcpService as never,
			configResolution as never,
			new NullLogService(),
		));

		const firedEntries: string[] = [];
		disposables.add(service.onDidChangeServerStatus(entry => { firedEntries.push(entry.name); }));

		const dummyConfig: ResolvedConfig = { mcpServers: [], agents: [], skills: [], disabled: { mcpServers: [], agents: [] } };
		_onDidChangeResolved.fire(dummyConfig);

		assert.ok(firedEntries.includes('srv'));
	});

	test('toggleServerDisabled delegates to configResolution', async () => {
		const mcpService = createMockMcpService([]);
		let calledWith: { name: string; disabled: boolean } | undefined;
		const _onDidChangeResolved = disposables.add(new Emitter<ResolvedConfig>());
		const configResolution = {
			_serviceBrand: undefined,
			onDidChangeResolved: _onDidChangeResolved.event,
			resolve: async () => ({ mcpServers: [], agents: [], skills: [], disabled: { mcpServers: [], agents: [] } }),
			getCached: () => undefined,
			setMcpServerDisabled: async (name: string, disabled: boolean) => { calledWith = { name, disabled }; },
			setAgentDisabled: async () => undefined,
		};
		const service = disposables.add(new ForgeMcpService(
			mcpService as never,
			configResolution as never,
			new NullLogService(),
		));

		await service.toggleServerDisabled('my-server', true);
		assert.deepStrictEqual(calledWith, { name: 'my-server', disabled: true });
	});

	test('listTools returns both entries when two servers have tools with the same name', async () => {
		const serverA = createMockMcpServer('server-a', McpConnectionState.Running, [
			{ name: 'read_file', description: 'Read from A', inputSchema: {} },
		]);
		const serverB = createMockMcpServer('server-b', McpConnectionState.Running, [
			{ name: 'read_file', description: 'Read from B', inputSchema: {} },
		]);
		const mcpService = createMockMcpService([serverA, serverB]);
		const configResolution = createMockConfigResolution();
		const service = disposables.add(new ForgeMcpService(
			mcpService as never,
			configResolution as never,
			new NullLogService(),
		));

		const tools = await service.listTools();
		// No deduplication — both entries appear
		assert.strictEqual(tools.length, 2);
		assert.ok(tools.every(t => t.name === 'read_file'));
	});

	test('disposed onDidChangeTools listener is not called after disposal', () => {
		const mcpService = createMockMcpService([]);
		const _onDidChangeResolved = disposables.add(new Emitter<ResolvedConfig>());
		const configResolution = {
			_serviceBrand: undefined,
			onDidChangeResolved: _onDidChangeResolved.event,
			resolve: async () => ({ mcpServers: [], agents: [], skills: [], disabled: { mcpServers: [], agents: [] } }),
			getCached: () => undefined,
			setMcpServerDisabled: async () => undefined,
			setAgentDisabled: async () => undefined,
		};
		const service = disposables.add(new ForgeMcpService(
			mcpService as never,
			configResolution as never,
			new NullLogService(),
		));

		let callCount = 0;
		const sub = service.onDidChangeTools(() => { callCount++; });
		sub.dispose();

		const dummyConfig: ResolvedConfig = { mcpServers: [], agents: [], skills: [], disabled: { mcpServers: [], agents: [] } };
		_onDidChangeResolved.fire(dummyConfig);

		assert.strictEqual(callCount, 0);
	});

	test('listTools returns new tool after config resolution fires with new server', async () => {
		let currentServers: ReturnType<typeof createMockMcpServer>[] = [];
		const dynamicMcpService = {
			...createMockMcpService([]),
			servers: { get: () => currentServers, read: () => currentServers } as never,
		};

		const _onDidChangeResolved = disposables.add(new Emitter<ResolvedConfig>());
		const configResolution = {
			_serviceBrand: undefined,
			onDidChangeResolved: _onDidChangeResolved.event,
			resolve: async () => ({ mcpServers: [], agents: [], skills: [], disabled: { mcpServers: [], agents: [] } }),
			getCached: () => undefined,
			setMcpServerDisabled: async () => undefined,
			setAgentDisabled: async () => undefined,
		};

		const service = disposables.add(new ForgeMcpService(
			dynamicMcpService as never,
			configResolution as never,
			new NullLogService(),
		));

		assert.strictEqual((await service.listTools()).length, 0);

		currentServers = [createMockMcpServer('new-server', McpConnectionState.Running, [
			{ name: 'grep_file', description: 'Grep a file', inputSchema: {} },
		])];
		_onDidChangeResolved.fire({ mcpServers: [], agents: [], skills: [], disabled: { mcpServers: [], agents: [] } });

		const tools = await service.listTools();
		assert.strictEqual(tools.length, 1);
		assert.strictEqual(tools[0].name, 'grep_file');
	});
});
