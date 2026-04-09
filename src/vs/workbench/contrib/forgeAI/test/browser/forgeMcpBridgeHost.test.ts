import assert from 'assert';
import { DisposableStore } from '../../../../../base/common/lifecycle.js';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import { ForgeMcpBridgeHost } from '../../browser/forgeMcpBridgeHost.js';

/** Minimal mock of VS Code's IMcpTool shape (just what ForgeMcpBridgeHost uses). */
function createMockTool(name: string, description: string, inputSchema: Record<string, unknown>) {
	return {
		definition: { name, description, inputSchema },
		call: async (params: Record<string, unknown>) => ({
			content: [{ type: 'text' as const, text: `result:${JSON.stringify(params)}` }],
			isError: false,
		}),
		// Additional VS Code IMcpTool fields not used by the bridge
		id: name,
		referenceName: name,
		icons: {},
		visibility: 3,
		callWithProgress: async () => ({ content: [], isError: false }),
	};
}

/** Minimal mock of a VS Code IMcpServer shape (just what ForgeMcpBridgeHost uses). */
function createMockVSCodeServer(id: string, label: string, connectionState: number, tools: ReturnType<typeof createMockTool>[]) {
	return {
		definition: { id, label },
		connectionState: { get: () => ({ state: connectionState }) },
		tools: { get: () => tools },
		readDefinitions: () => ({ get: () => ({ server: undefined }) }),
	};
}

/** Minimal mock of VS Code's IMcpService shape (just what ForgeMcpBridgeHost uses). */
function createMockVSCodeMcpService(servers: ReturnType<typeof createMockVSCodeServer>[]) {
	return {
		_serviceBrand: undefined,
		servers: { get: () => servers },
	};
}

suite('ForgeMcpBridgeHost', () => {
	let disposables: DisposableStore;

	setup(() => {
		disposables = new DisposableStore();
	});

	teardown(() => {
		disposables.dispose();
	});

	ensureNoDisposablesAreLeakedInTestSuite();

	test('servers.get() returns mapped server list', () => {
		const vscodeMcp = createMockVSCodeMcpService([
			createMockVSCodeServer('srv-1', 'filesystem', 2, []),
			createMockVSCodeServer('srv-2', 'github', 2, []),
		]);

		const host = disposables.add(new ForgeMcpBridgeHost(vscodeMcp as never));
		const servers = host.servers.get();

		assert.strictEqual(servers.length, 2);
		assert.strictEqual(servers[0].definition.id, 'srv-1');
		assert.strictEqual(servers[0].definition.label, 'filesystem');
		assert.strictEqual(servers[1].definition.label, 'github');
	});

	test('server connectionState.get().state is preserved', () => {
		const vscodeMcp = createMockVSCodeMcpService([
			createMockVSCodeServer('srv', 'srv', 2 /* Running */, []),
		]);

		const host = disposables.add(new ForgeMcpBridgeHost(vscodeMcp as never));
		const state = host.servers.get()[0].connectionState.get();

		assert.strictEqual(state.state, 2);
	});

	test('tool definitions are correctly mapped', () => {
		const vscodeMcp = createMockVSCodeMcpService([
			createMockVSCodeServer('fs', 'filesystem', 2, [
				createMockTool('read_file', 'Read a file', { type: 'object', properties: { path: { type: 'string' } } }),
				createMockTool('write_file', 'Write a file', { type: 'object', properties: { path: { type: 'string' }, content: { type: 'string' } } }),
			]),
		]);

		const host = disposables.add(new ForgeMcpBridgeHost(vscodeMcp as never));
		const tools = host.servers.get()[0].tools.get();

		assert.strictEqual(tools.length, 2);
		assert.strictEqual(tools[0].definition.name, 'read_file');
		assert.strictEqual(tools[0].definition.description, 'Read a file');
		assert.strictEqual(tools[1].definition.name, 'write_file');
	});

	test('tool inputSchema is preserved through the adapter', () => {
		const schema = { type: 'object', properties: { path: { type: 'string' }, encoding: { type: 'string' } } };
		const vscodeMcp = createMockVSCodeMcpService([
			createMockVSCodeServer('fs', 'filesystem', 2, [
				createMockTool('read_file', 'Read', schema),
			]),
		]);

		const host = disposables.add(new ForgeMcpBridgeHost(vscodeMcp as never));
		const toolSchema = host.servers.get()[0].tools.get()[0].definition.inputSchema;

		assert.deepStrictEqual(toolSchema, schema);
	});

	test('tool.call() delegates to the underlying VS Code tool', async () => {
		const vscodeMcp = createMockVSCodeMcpService([
			createMockVSCodeServer('fs', 'filesystem', 2, [
				createMockTool('read_file', 'Read', {}),
			]),
		]);

		const host = disposables.add(new ForgeMcpBridgeHost(vscodeMcp as never));
		const tool = host.servers.get()[0].tools.get()[0];

		const result = await tool.call({ path: '/tmp/hello.txt' });

		assert.ok(!result.isError);
		assert.ok(result.content.length > 0);
		assert.ok(result.content[0].text?.includes('/tmp/hello.txt'));
	});

	test('servers.get() returns empty array when no servers registered', () => {
		const vscodeMcp = createMockVSCodeMcpService([]);
		const host = disposables.add(new ForgeMcpBridgeHost(vscodeMcp as never));

		assert.deepStrictEqual(host.servers.get(), []);
	});

	test('servers.get() is evaluated lazily on each call', () => {
		const serverList: ReturnType<typeof createMockVSCodeServer>[] = [];
		const vscodeMcp = { _serviceBrand: undefined, servers: { get: () => serverList } };

		const host = disposables.add(new ForgeMcpBridgeHost(vscodeMcp as never));

		assert.strictEqual(host.servers.get().length, 0);

		serverList.push(createMockVSCodeServer('new-srv', 'new-server', 2, []));

		assert.strictEqual(host.servers.get().length, 1);
	});
});
