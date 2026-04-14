import assert from 'assert';
import { DisposableStore } from '../../../../../base/common/lifecycle.js';
import { Emitter, Event } from '../../../../../base/common/event.js';
import { URI } from '../../../../../base/common/uri.js';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import { NullLogService } from '../../../../../platform/log/common/log.js';
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

	// Global forge.json is at userRoamingDataHome/forge/forge.json
	const GLOBAL_FORGE_JSON = '/home/user/.config/forge/forge.json';

	function createMockFileService(files: Record<string, string>, fileChangeEmitter?: Emitter<void>) {
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
			watch: (): { dispose: () => void } => ({ dispose: () => { } }),
			onDidFilesChange: (fileChangeEmitter ? fileChangeEmitter.event : Event.None) as never,
			writeFile: async (uri: { path: string }, data: { toString: () => string }) => {
				files[uri.path] = data.toString();
			},
		};
	}

	function createMockEnvironmentService(roamingDataHome: string = '/home/user/.config') {
		return {
			_serviceBrand: undefined,
			userRoamingDataHome: URI.file(roamingDataHome),
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
			createMockEnvironmentService() as never,
			createMockPathService() as never,
			new NullLogService(),
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
			createMockEnvironmentService() as never,
			createMockPathService() as never,
			new NullLogService(),
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
			}),
			'/workspace/forge.json': JSON.stringify({
				disabled: { mcpServers: ['drop'], agents: [] }
			})
		};
		const service = disposables.add(new ForgeConfigResolutionService(
			createMockFileService(files) as never,
			createMockEnvironmentService() as never,
			createMockPathService() as never,
			new NullLogService(),
		));
		const resolved = await service.resolve('/workspace');
		assert.strictEqual(resolved.mcpServers.length, 1);
		assert.strictEqual(resolved.mcpServers[0].name, 'keep');
	});

	test('configPaths.mcp adds additional MCP sources', async () => {
		const files: Record<string, string> = {
			'/shared/team/.mcp.json': JSON.stringify({
				mcpServers: { team_server: { command: 'team-cmd', args: [] } }
			}),
			'/workspace/forge.json': JSON.stringify({
				configPaths: { mcp: ['/shared/team/'] }
			})
		};
		const service = disposables.add(new ForgeConfigResolutionService(
			createMockFileService(files) as never,
			createMockEnvironmentService() as never,
			createMockPathService() as never,
			new NullLogService(),
		));
		const resolved = await service.resolve('/workspace');
		assert.strictEqual(resolved.mcpServers.length, 1);
		assert.strictEqual(resolved.mcpServers[0].name, 'team_server');
	});

	test('getCached returns undefined before first resolve', () => {
		const service = disposables.add(new ForgeConfigResolutionService(
			createMockFileService({}) as never,
			createMockEnvironmentService() as never,
			createMockPathService() as never,
			new NullLogService(),
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
			createMockEnvironmentService() as never,
			createMockPathService() as never,
			new NullLogService(),
		));
		await service.resolve('/workspace');
		const cached = service.getCached();
		assert.ok(cached);
		assert.strictEqual(cached.mcpServers.length, 1);
	});

	test('no config files exist returns empty config without throwing', async () => {
		const service = disposables.add(new ForgeConfigResolutionService(
			createMockFileService({}) as never,
			createMockEnvironmentService() as never,
			createMockPathService() as never,
			new NullLogService(),
		));
		const resolved = await service.resolve('/workspace');
		assert.strictEqual(resolved.mcpServers.length, 0);
		assert.strictEqual(resolved.agents.length, 0);
		assert.strictEqual(resolved.skills.length, 0);
	});

	test('agents dir with multiple .md files loads all agents', async () => {
		const files: Record<string, string> = {
			'/home/user/.agents/developer.md': [
				'---', 'name: developer', 'description: Writes code', '---', 'You write code.',
			].join('\n'),
			'/home/user/.agents/reviewer.md': [
				'---', 'name: reviewer', 'description: Reviews code', '---', 'You review code.',
			].join('\n'),
			'/home/user/.agents/tester.md': [
				'---', 'name: tester', 'description: Writes tests', '---', 'You write tests.',
			].join('\n'),
		};
		const service = disposables.add(new ForgeConfigResolutionService(
			createMockFileService(files) as never,
			createMockEnvironmentService() as never,
			createMockPathService() as never,
			new NullLogService(),
		));
		const resolved = await service.resolve('/workspace');
		assert.strictEqual(resolved.agents.length, 3);
		const names = resolved.agents.map(a => a.name).sort();
		assert.deepStrictEqual(names, ['developer', 'reviewer', 'tester']);
	});

	test('agent file with invalid frontmatter is skipped silently', async () => {
		const files: Record<string, string> = {
			'/home/user/.agents/valid.md': [
				'---', 'name: valid-agent', '---', 'System prompt.',
			].join('\n'),
			'/home/user/.agents/invalid.md': 'no frontmatter at all',
			'/home/user/.agents/no-name.md': [
				'---', 'description: Missing name', '---', 'prompt',
			].join('\n'),
		};
		const service = disposables.add(new ForgeConfigResolutionService(
			createMockFileService(files) as never,
			createMockEnvironmentService() as never,
			createMockPathService() as never,
			new NullLogService(),
		));
		const resolved = await service.resolve('/workspace');
		assert.strictEqual(resolved.agents.length, 1);
		assert.strictEqual(resolved.agents[0].name, 'valid-agent');
	});

	test('disabled config excludes agents from resolved result', async () => {
		const files: Record<string, string> = {
			'/home/user/.agents/keep.md': [
				'---', 'name: keep', '---', 'Keep this agent.',
			].join('\n'),
			'/home/user/.agents/drop.md': [
				'---', 'name: drop', '---', 'Drop this agent.',
			].join('\n'),
			'/workspace/forge.json': JSON.stringify({
				disabled: { mcpServers: [], agents: ['drop'] }
			})
		};
		const service = disposables.add(new ForgeConfigResolutionService(
			createMockFileService(files) as never,
			createMockEnvironmentService() as never,
			createMockPathService() as never,
			new NullLogService(),
		));
		const resolved = await service.resolve('/workspace');
		assert.strictEqual(resolved.agents.length, 1);
		assert.strictEqual(resolved.agents[0].name, 'keep');
	});

	test('missing file at configPaths.mcp is skipped gracefully', async () => {
		const files: Record<string, string> = {
			'/workspace/forge.json': JSON.stringify({
				configPaths: { mcp: ['/nonexistent/path/'] }
			})
		};
		const service = disposables.add(new ForgeConfigResolutionService(
			createMockFileService(files) as never,
			createMockEnvironmentService() as never,
			createMockPathService() as never,
			new NullLogService(),
		));
		// Should not throw; should return empty config
		const resolved = await service.resolve('/workspace');
		assert.strictEqual(resolved.mcpServers.length, 0);
	});

	test('project agents override global agents with the same name', async () => {
		const files: Record<string, string> = {
			'/home/user/.agents/dev.md': [
				'---', 'name: dev', 'description: Global developer', '---', 'Global prompt.',
			].join('\n'),
			'/workspace/.agents/dev.md': [
				'---', 'name: dev', 'description: Project developer', '---', 'Project prompt.',
			].join('\n'),
		};
		const service = disposables.add(new ForgeConfigResolutionService(
			createMockFileService(files) as never,
			createMockEnvironmentService() as never,
			createMockPathService() as never,
			new NullLogService(),
		));
		const resolved = await service.resolve('/workspace');
		assert.strictEqual(resolved.agents.length, 1);
		assert.strictEqual(resolved.agents[0].description, 'Project developer');
		assert.strictEqual(resolved.agents[0].systemPrompt, 'Project prompt.');
	});

	test('onDidChangeResolved fires after resolve completes', async () => {
		const service = disposables.add(new ForgeConfigResolutionService(
			createMockFileService({}) as never,
			createMockEnvironmentService() as never,
			createMockPathService() as never,
			new NullLogService(),
		));
		let fired = false;
		disposables.add(service.onDidChangeResolved(() => { fired = true; }));
		await service.resolve('/workspace');
		assert.strictEqual(fired, true);
	});

	test('setMcpServerDisabled adds server to disabled list', async () => {
		const files: Record<string, string> = {};
		const service = disposables.add(new ForgeConfigResolutionService(
			createMockFileService(files) as never,
			createMockEnvironmentService() as never,
			createMockPathService() as never,
			new NullLogService(),
		));
		await service.setMcpServerDisabled('my-server', true);
		const written = files[GLOBAL_FORGE_JSON];
		assert.ok(written, 'forge.json should have been written');
		const parsed = JSON.parse(written) as { disabled: { mcpServers: string[] } };
		assert.ok(parsed.disabled.mcpServers.includes('my-server'));
	});

	test('setMcpServerDisabled removing server is idempotent', async () => {
		const files: Record<string, string> = {
			[GLOBAL_FORGE_JSON]: JSON.stringify({
				disabled: { mcpServers: ['my-server'], agents: [] }
			})
		};
		const service = disposables.add(new ForgeConfigResolutionService(
			createMockFileService(files) as never,
			createMockEnvironmentService() as never,
			createMockPathService() as never,
			new NullLogService(),
		));
		await service.setMcpServerDisabled('my-server', false);
		const written = files[GLOBAL_FORGE_JSON];
		const parsed = JSON.parse(written) as { disabled: { mcpServers: string[] } };
		assert.ok(!parsed.disabled.mcpServers.includes('my-server'));
	});

	test('setMcpServerDisabled disabling twice does not duplicate', async () => {
		const files: Record<string, string> = {
			[GLOBAL_FORGE_JSON]: JSON.stringify({
				disabled: { mcpServers: ['existing'], agents: [] }
			})
		};
		const service = disposables.add(new ForgeConfigResolutionService(
			createMockFileService(files) as never,
			createMockEnvironmentService() as never,
			createMockPathService() as never,
			new NullLogService(),
		));
		await service.setMcpServerDisabled('existing', true);
		const written = files[GLOBAL_FORGE_JSON];
		const parsed = JSON.parse(written) as { disabled: { mcpServers: string[] } };
		assert.strictEqual(parsed.disabled.mcpServers.filter(s => s === 'existing').length, 1);
	});

	test('setAgentDisabled adds agent to disabled list', async () => {
		const files: Record<string, string> = {};
		const service = disposables.add(new ForgeConfigResolutionService(
			createMockFileService(files) as never,
			createMockEnvironmentService() as never,
			createMockPathService() as never,
			new NullLogService(),
		));
		await service.setAgentDisabled('developer', true);
		const written = files[GLOBAL_FORGE_JSON];
		assert.ok(written, 'forge.json should have been written');
		const parsed = JSON.parse(written) as { disabled: { agents: string[] } };
		assert.ok(parsed.disabled.agents.includes('developer'));
	});

	test('setAgentDisabled re-enabling removes agent from disabled list', async () => {
		const files: Record<string, string> = {
			[GLOBAL_FORGE_JSON]: JSON.stringify({
				disabled: { mcpServers: [], agents: ['developer'] }
			})
		};
		const service = disposables.add(new ForgeConfigResolutionService(
			createMockFileService(files) as never,
			createMockEnvironmentService() as never,
			createMockPathService() as never,
			new NullLogService(),
		));
		await service.setAgentDisabled('developer', false);
		const written = files[GLOBAL_FORGE_JSON];
		const parsed = JSON.parse(written) as { disabled: { agents: string[] } };
		assert.ok(!parsed.disabled.agents.includes('developer'));
	});

	test('file watcher triggers onDidChangeResolved after debounce', async () => {
		const fileChangeEmitter = disposables.add(new Emitter<void>());
		const service = disposables.add(new ForgeConfigResolutionService(
			createMockFileService({}, fileChangeEmitter) as never,
			createMockEnvironmentService() as never,
			createMockPathService() as never,
			new NullLogService(),
		));

		await service.resolve('/workspace');

		let fireCount = 0;
		disposables.add(service.onDidChangeResolved(() => { fireCount++; }));

		fileChangeEmitter.fire();

		await new Promise(resolve => setTimeout(resolve, 500));

		assert.strictEqual(fireCount, 1);
	});

	test('rapid file changes are debounced to a single resolve', async () => {
		const fileChangeEmitter = disposables.add(new Emitter<void>());
		const service = disposables.add(new ForgeConfigResolutionService(
			createMockFileService({}, fileChangeEmitter) as never,
			createMockEnvironmentService() as never,
			createMockPathService() as never,
			new NullLogService(),
		));

		await service.resolve('/workspace');

		let fireCount = 0;
		disposables.add(service.onDidChangeResolved(() => { fireCount++; }));

		for (let i = 0; i < 5; i++) {
			fileChangeEmitter.fire();
		}

		await new Promise(resolve => setTimeout(resolve, 500));

		assert.strictEqual(fireCount, 1);
	});

	test('concurrent calls to resolve() both complete and return consistent results', async () => {
		const files: Record<string, string> = {
			'/home/user/.mcp.json': JSON.stringify({
				mcpServers: { concurrent: { command: 'cmd', args: [] } }
			})
		};
		const service = disposables.add(new ForgeConfigResolutionService(
			createMockFileService(files) as never,
			createMockEnvironmentService() as never,
			createMockPathService() as never,
			new NullLogService(),
		));

		let resolveCount = 0;
		disposables.add(service.onDidChangeResolved(() => { resolveCount++; }));

		const [r1, r2] = await Promise.all([
			service.resolve('/workspace'),
			service.resolve('/workspace'),
		]);

		assert.strictEqual(r1.mcpServers.length, 1);
		assert.strictEqual(r2.mcpServers.length, 1);
		assert.ok(resolveCount >= 1);
	});
});
