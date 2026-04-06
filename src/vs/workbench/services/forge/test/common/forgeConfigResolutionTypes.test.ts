import assert from 'assert';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import {
	type McpJsonConfig,
	type McpServerEntry,
	type AgentDefinition,
	type SkillDefinition,
	type ConfigPaths,
	type DisabledConfig,
	type ResolvedConfig,
	parseMcpJson,
	parseAgentMarkdown
} from '../../common/forgeConfigResolutionTypes.js';

suite('ForgeConfigResolutionTypes', () => {
	ensureNoDisposablesAreLeakedInTestSuite();

	suite('McpJsonConfig', () => {
		test('parseMcpJson parses standard .mcp.json format', () => {
			const raw = {
				mcpServers: {
					filesystem: {
						command: 'npx',
						args: ['-y', '@modelcontextprotocol/server-filesystem', '/tmp'],
						env: { GITHUB_TOKEN: '${env:GITHUB_TOKEN}' }
					}
				}
			};
			const result = parseMcpJson(raw);
			assert.ok(result.valid);
			assert.strictEqual(result.servers.length, 1);
			assert.strictEqual(result.servers[0].name, 'filesystem');
			assert.strictEqual(result.servers[0].command, 'npx');
			assert.deepStrictEqual(result.servers[0].args, ['-y', '@modelcontextprotocol/server-filesystem', '/tmp']);
			assert.strictEqual(result.servers[0].env!['GITHUB_TOKEN'], '${env:GITHUB_TOKEN}');
		});

		test('parseMcpJson handles HTTP/SSE servers', () => {
			const raw = {
				mcpServers: {
					'remote-api': {
						url: 'https://mcp.example.com/sse',
						headers: { Authorization: 'Bearer token' }
					}
				}
			};
			const result = parseMcpJson(raw);
			assert.ok(result.valid);
			assert.strictEqual(result.servers[0].name, 'remote-api');
			assert.strictEqual(result.servers[0].url, 'https://mcp.example.com/sse');
			assert.strictEqual(result.servers[0].headers!['Authorization'], 'Bearer token');
		});

		test('parseMcpJson returns invalid for missing mcpServers key', () => {
			const result = parseMcpJson({});
			assert.ok(!result.valid);
		});

		test('parseMcpJson handles empty mcpServers', () => {
			const result = parseMcpJson({ mcpServers: {} });
			assert.ok(result.valid);
			assert.strictEqual(result.servers.length, 0);
		});
	});

	suite('AgentDefinition', () => {
		test('parseAgentMarkdown extracts frontmatter and body', () => {
			const content = [
				'---',
				'name: code-reviewer',
				'description: Reviews code for style and bugs',
				'tools: [filesystem, github]',
				'maxTurns: 10',
				'---',
				'',
				'You are a code reviewer. Focus on correctness.',
			].join('\n');
			const result = parseAgentMarkdown(content);
			assert.ok(result);
			assert.strictEqual(result.name, 'code-reviewer');
			assert.strictEqual(result.description, 'Reviews code for style and bugs');
			assert.deepStrictEqual(result.tools, ['filesystem', 'github']);
			assert.strictEqual(result.maxTurns, 10);
			assert.strictEqual(result.systemPrompt, 'You are a code reviewer. Focus on correctness.');
		});

		test('parseAgentMarkdown returns null for missing frontmatter', () => {
			const result = parseAgentMarkdown('Just some text without frontmatter');
			assert.strictEqual(result, null);
		});

		test('parseAgentMarkdown returns null for missing name', () => {
			const content = [
				'---',
				'description: No name field',
				'---',
				'body text',
			].join('\n');
			const result = parseAgentMarkdown(content);
			assert.strictEqual(result, null);
		});

		test('parseAgentMarkdown uses empty string as fallback description', () => {
			const content = [
				'---',
				'name: my-agent',
				'---',
				'System prompt here.',
			].join('\n');
			const result = parseAgentMarkdown(content);
			assert.ok(result);
			assert.strictEqual(result.description, '');
			assert.strictEqual(result.systemPrompt, 'System prompt here.');
		});
	});

	suite('DisabledConfig', () => {
		test('DisabledConfig defaults to empty arrays', () => {
			const config: DisabledConfig = {
				mcpServers: [],
				agents: []
			};
			assert.deepStrictEqual(config.mcpServers, []);
			assert.deepStrictEqual(config.agents, []);
		});
	});

	suite('ConfigPaths', () => {
		test('ConfigPaths fields are all optional arrays', () => {
			const paths: ConfigPaths = {};
			assert.strictEqual(paths.mcp, undefined);
			assert.strictEqual(paths.agents, undefined);
			assert.strictEqual(paths.skills, undefined);
		});

		test('ConfigPaths accepts string arrays', () => {
			const paths: ConfigPaths = {
				mcp: ['~/shared-mcp/'],
				agents: ['~/my-agents/', '/team/agents/'],
				skills: ['~/my-skills/']
			};
			assert.strictEqual(paths.mcp!.length, 1);
			assert.strictEqual(paths.agents!.length, 2);
			assert.strictEqual(paths.skills!.length, 1);
		});
	});

	suite('parseYamlFrontmatter', () => {
		// parseYamlFrontmatter is not exported but exercised through parseAgentMarkdown.
		// These tests drive it via that public surface.

		test('inline array [a, b, c] is parsed as string[]', () => {
			const content = [
				'---',
				'name: agent',
				'tools: [read_file, write_file, search]',
				'---',
				'prompt',
			].join('\n');
			const result = parseAgentMarkdown(content);
			assert.ok(result);
			assert.deepStrictEqual(result.tools, ['read_file', 'write_file', 'search']);
		});

		test('numeric value is parsed as number (maxTurns)', () => {
			const content = [
				'---',
				'name: agent',
				'maxTurns: 42',
				'---',
				'prompt',
			].join('\n');
			const result = parseAgentMarkdown(content);
			assert.ok(result);
			assert.strictEqual(result.maxTurns, 42);
		});

		test('unknown frontmatter keys are silently ignored', () => {
			const content = [
				'---',
				'name: agent',
				'unknownKey: someValue',
				'anotherKey: 999',
				'---',
				'prompt',
			].join('\n');
			const result = parseAgentMarkdown(content);
			assert.ok(result);
			assert.strictEqual(result.name, 'agent');
		});

		test('string values have surrounding quotes stripped', () => {
			const content = [
				'---',
				'name: "quoted-agent"',
				'description: \'single-quoted\'',
				'---',
				'prompt',
			].join('\n');
			const result = parseAgentMarkdown(content);
			assert.ok(result);
			assert.strictEqual(result.name, 'quoted-agent');
			assert.strictEqual(result.description, 'single-quoted');
		});
	});

	suite('parseAgentMarkdown edge cases', () => {
		test('frontmatter only with empty body yields empty systemPrompt', () => {
			const content = [
				'---',
				'name: minimal',
				'---',
			].join('\n');
			const result = parseAgentMarkdown(content);
			assert.ok(result);
			assert.strictEqual(result.systemPrompt, '');
		});

		test('provider and model fields are parsed when present', () => {
			const content = [
				'---',
				'name: specific-agent',
				'provider: anthropic',
				'model: claude-sonnet-4-6',
				'---',
				'System prompt.',
			].join('\n');
			const result = parseAgentMarkdown(content);
			assert.ok(result);
			assert.strictEqual(result.provider, 'anthropic');
			assert.strictEqual(result.model, 'claude-sonnet-4-6');
		});

		test('missing provider and model fields are undefined', () => {
			const content = [
				'---',
				'name: no-provider',
				'---',
				'prompt',
			].join('\n');
			const result = parseAgentMarkdown(content);
			assert.ok(result);
			assert.strictEqual(result.provider, undefined);
			assert.strictEqual(result.model, undefined);
		});

		test('tools field absent leaves tools undefined', () => {
			const content = [
				'---',
				'name: no-tools',
				'---',
				'prompt',
			].join('\n');
			const result = parseAgentMarkdown(content);
			assert.ok(result);
			assert.strictEqual(result.tools, undefined);
		});
	});

	suite('parseMcpJson additional cases', () => {
		test('null input returns invalid', () => {
			const result = parseMcpJson(null);
			assert.ok(!result.valid);
		});

		test('array input returns invalid', () => {
			const result = parseMcpJson([]);
			assert.ok(!result.valid);
		});

		test('mixed stdio and HTTP servers both parsed', () => {
			const raw = {
				mcpServers: {
					local: { command: 'npx', args: ['-y', 'server'] },
					remote: { url: 'https://mcp.example.com', headers: { Authorization: 'Bearer x' } },
				}
			};
			const result = parseMcpJson(raw);
			assert.ok(result.valid);
			assert.strictEqual(result.servers.length, 2);
			const local = result.servers.find(s => s.name === 'local');
			const remote = result.servers.find(s => s.name === 'remote');
			assert.ok(local);
			assert.ok(remote);
			assert.strictEqual(local.command, 'npx');
			assert.strictEqual(remote.url, 'https://mcp.example.com');
		});
	});

	// Suppress unused import warnings — these types are tested via shape assertions
	const _typecheck: [McpJsonConfig, McpServerEntry, AgentDefinition, SkillDefinition, ResolvedConfig] = undefined!;
	void _typecheck;
});
