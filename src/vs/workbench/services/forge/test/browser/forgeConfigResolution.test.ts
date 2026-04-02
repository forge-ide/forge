import assert from 'assert';
import { DisposableStore } from '../../../../../base/common/lifecycle.js';
import { URI } from '../../../../../base/common/uri.js';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import { ForgeConfigResolutionService } from '../../browser/forgeConfigResolution.js';

suite('ForgeConfigResolutionService', () => {

	let disposables: DisposableStore;

	setup(() => {
		disposables = new DisposableStore();
	});

	teardown(() => {
		disposables.dispose();
	});

	ensureNoDisposablesAreLeakedInTestSuite();

	function createMockFileService(files: Record<string, string>) {
		return {
			_serviceBrand: undefined,
			readFile: async (uri: { path: string }) => {
				const content = files[uri.path];
				if (content === undefined) {
					throw new Error(`File not found: ${uri.path}`);
				}
				return { value: { toString: () => content } };
			},
			resolve: async (uri: { path: string }) => ({
				children: Object.keys(files)
					.filter(p => {
						const prefix = uri.path.endsWith('/') ? uri.path : uri.path + '/';
						return p.startsWith(prefix) && !p.slice(prefix.length).includes('/');
					})
					.map(p => ({
						resource: { path: p, toString: () => p },
						isDirectory: false,
						name: p.split('/').pop()
					}))
			}),
			exists: async (uri: { path: string }) => Object.prototype.hasOwnProperty.call(files, uri.path),
		};
	}

	function createMockConfigService(config: Record<string, unknown> = {}) {
		return {
			_serviceBrand: undefined,
			getConfig: () => ({
				defaultProvider: 'anthropic',
				providers: [],
				configPaths: config['configPaths'],
				disabled: config['disabled'],
			}),
			updateConfig: async (partial: Record<string, unknown>) => {
				Object.assign(config, partial);
			},
		};
	}

	function createMockPathService(homePath: string = '/home/user') {
		return {
			_serviceBrand: undefined,
			userHome: async () => URI.file(homePath),
			resolvedUserHome: URI.file(homePath),
			hasValidBasename: async () => true,
		};
	}

	test('resolves MCP servers from global .mcp.json', async () => {
		const files: Record<string, string> = {
			'/home/user/.mcp.json': JSON.stringify({
				mcpServers: {
					filesystem: { command: 'npx', args: ['-y', '@mcp/server-fs'] }
				}
			})
		};
		const service = disposables.add(new ForgeConfigResolutionService(
			createMockFileService(files) as never,
			createMockConfigService() as never,
			createMockPathService() as never,
		));
		const resolved = await service.resolve('/workspace');
		assert.strictEqual(resolved.mcpServers.length, 1);
		assert.strictEqual(resolved.mcpServers[0].name, 'filesystem');
	});

	test('project .mcp.json overrides global on name conflict', async () => {
		const files: Record<string, string> = {
			'/home/user/.mcp.json': JSON.stringify({
				mcpServers: { fs: { command: 'global-cmd', args: [] } }
			}),
			'/workspace/.mcp.json': JSON.stringify({
				mcpServers: { fs: { command: 'project-cmd', args: [] } }
			})
		};
		const service = disposables.add(new ForgeConfigResolutionService(
			createMockFileService(files) as never,
			createMockConfigService() as never,
			createMockPathService() as never,
		));
		const resolved = await service.resolve('/workspace');
		assert.strictEqual(resolved.mcpServers.length, 1);
		assert.strictEqual(resolved.mcpServers[0].command, 'project-cmd');
	});

	test('disabled mcpServers are excluded from resolved config', async () => {
		const files: Record<string, string> = {
			'/home/user/.mcp.json': JSON.stringify({
				mcpServers: {
					keep: { command: 'keep-cmd', args: [] },
					drop: { command: 'drop-cmd', args: [] }
				}
			})
		};
		const service = disposables.add(new ForgeConfigResolutionService(
			createMockFileService(files) as never,
			createMockConfigService({ disabled: { mcpServers: ['drop'], agents: [] } }) as never,
			createMockPathService() as never,
		));
		const resolved = await service.resolve('/workspace');
		assert.strictEqual(resolved.mcpServers.length, 1);
		assert.strictEqual(resolved.mcpServers[0].name, 'keep');
	});

	test('configPaths.mcp adds additional MCP sources', async () => {
		const files: Record<string, string> = {
			'/shared/team/.mcp.json': JSON.stringify({
				mcpServers: { team_server: { command: 'team-cmd', args: [] } }
			})
		};
		const service = disposables.add(new ForgeConfigResolutionService(
			createMockFileService(files) as never,
			createMockConfigService({ configPaths: { mcp: ['/shared/team/'] } }) as never,
			createMockPathService() as never,
		));
		const resolved = await service.resolve('/workspace');
		assert.strictEqual(resolved.mcpServers.length, 1);
		assert.strictEqual(resolved.mcpServers[0].name, 'team_server');
	});

	test('getCached returns undefined before first resolve', () => {
		const service = disposables.add(new ForgeConfigResolutionService(
			createMockFileService({}) as never,
			createMockConfigService() as never,
			createMockPathService() as never,
		));
		assert.strictEqual(service.getCached(), undefined);
	});

	test('getCached returns last resolved config', async () => {
		const files: Record<string, string> = {
			'/home/user/.mcp.json': JSON.stringify({
				mcpServers: { srv: { command: 'cmd', args: [] } }
			})
		};
		const service = disposables.add(new ForgeConfigResolutionService(
			createMockFileService(files) as never,
			createMockConfigService() as never,
			createMockPathService() as never,
		));
		await service.resolve('/workspace');
		const cached = service.getCached();
		assert.ok(cached);
		assert.strictEqual(cached.mcpServers.length, 1);
	});
});
