import { Disposable } from '../../../../base/common/lifecycle.js';
import { Emitter, Event } from '../../../../base/common/event.js';
import { IFileService } from '../../../../platform/files/common/files.js';
import { URI } from '../../../../base/common/uri.js';
import { IForgeConfigResolutionService } from '../common/forgeConfigResolution.js';
import { IForgeConfigService } from '../common/forgeConfigService.js';
import {
	ResolvedConfig, McpServerEntry, AgentDefinition, SkillDefinition,
	DisabledConfig, parseMcpJson, parseAgentMarkdown
} from '../common/forgeConfigResolutionTypes.js';
import { parseSkillMarkdown } from '../common/forgeSkillTypes.js';
import { IPathService } from '../../path/common/pathService.js';

export class ForgeConfigResolutionService extends Disposable implements IForgeConfigResolutionService {
	declare readonly _serviceBrand: undefined;

	private readonly _onDidChangeResolved = this._register(new Emitter<ResolvedConfig>());
	readonly onDidChangeResolved: Event<ResolvedConfig> = this._onDidChangeResolved.event;

	private _cached: ResolvedConfig | undefined;

	constructor(
		@IFileService private readonly fileService: IFileService,
		@IForgeConfigService private readonly configService: IForgeConfigService,
		@IPathService private readonly pathService: IPathService,
	) {
		super();
	}

	getCached(): ResolvedConfig | undefined {
		return this._cached;
	}

	async resolve(workspaceRoot: string): Promise<ResolvedConfig> {
		const config = this.configService.getConfig();
		const homeUri = await this.pathService.userHome();
		const homePath = homeUri.path;
		const configPaths = config.configPaths;
		const disabled: DisabledConfig = config.disabled ?? { mcpServers: [], agents: [] };

		// --- Collect MCP servers ---
		const serverMap = new Map<string, McpServerEntry>();

		// 1. Global ~/.mcp.json
		await this.loadMcpJson(URI.joinPath(URI.file(homePath), '.mcp.json'), serverMap);

		// 2. configPaths.mcp entries
		if (configPaths?.mcp) {
			for (const dir of configPaths.mcp) {
				const resolved = this.expandHome(dir, homePath);
				await this.loadMcpJson(URI.joinPath(URI.file(resolved), '.mcp.json'), serverMap);
			}
		}

		// 3. Project .mcp.json (wins on conflict)
		await this.loadMcpJson(URI.joinPath(URI.file(workspaceRoot), '.mcp.json'), serverMap);

		// --- Collect agents ---
		const agentMap = new Map<string, AgentDefinition>();

		// 1. Global ~/.agents/
		await this.loadAgentsDir(URI.joinPath(URI.file(homePath), '.agents'), agentMap);

		// 2. configPaths.agents entries
		if (configPaths?.agents) {
			for (const dir of configPaths.agents) {
				const resolved = this.expandHome(dir, homePath);
				await this.loadAgentsDir(URI.file(resolved), agentMap);
			}
		}

		// 3. Project .agents/
		await this.loadAgentsDir(URI.joinPath(URI.file(workspaceRoot), '.agents'), agentMap);

		// --- Collect skills ---
		const skillMap = new Map<string, SkillDefinition>();

		// 1. Global ~/.skills/
		await this.loadSkillsDir(URI.joinPath(URI.file(homePath), '.skills'), skillMap);

		// 2. configPaths.skills entries
		if (configPaths?.skills) {
			for (const dir of configPaths.skills) {
				const resolved = this.expandHome(dir, homePath);
				await this.loadSkillsDir(URI.file(resolved), skillMap);
			}
		}

		// 3. Project .skills/
		await this.loadSkillsDir(URI.joinPath(URI.file(workspaceRoot), '.skills'), skillMap);

		// --- Apply disabled filter ---
		for (const name of disabled.mcpServers) {
			serverMap.delete(name);
		}
		for (const name of disabled.agents) {
			agentMap.delete(name);
		}

		const resolvedConfig: ResolvedConfig = {
			mcpServers: Array.from(serverMap.values()),
			agents: Array.from(agentMap.values()),
			skills: Array.from(skillMap.values()),
			disabled,
		};

		this._cached = resolvedConfig;
		this._onDidChangeResolved.fire(resolvedConfig);
		return resolvedConfig;
	}

	async setMcpServerDisabled(serverName: string, disabled: boolean): Promise<void> {
		const config = this.configService.getConfig();
		const current = config.disabled ?? { mcpServers: [], agents: [] };
		const mcpSet = new Set(current.mcpServers);

		if (disabled) {
			mcpSet.add(serverName);
		} else {
			mcpSet.delete(serverName);
		}

		await this.configService.updateConfig({
			disabled: { mcpServers: Array.from(mcpSet), agents: current.agents }
		});
	}

	async setAgentDisabled(agentName: string, disabled: boolean): Promise<void> {
		const config = this.configService.getConfig();
		const current = config.disabled ?? { mcpServers: [], agents: [] };
		const agentSet = new Set(current.agents);

		if (disabled) {
			agentSet.add(agentName);
		} else {
			agentSet.delete(agentName);
		}

		await this.configService.updateConfig({
			disabled: { mcpServers: current.mcpServers, agents: Array.from(agentSet) }
		});
	}

	// --- Private helpers ---

	private async loadMcpJson(uri: URI, map: Map<string, McpServerEntry>): Promise<void> {
		try {
			const content = await this.fileService.readFile(uri);
			const raw = JSON.parse(content.value.toString());
			const result = parseMcpJson(raw);
			if (result.valid) {
				for (const server of result.servers) {
					map.set(server.name, server);
				}
			}
		} catch {
			// File not found or parse error — skip silently
		}
	}

	private async loadAgentsDir(uri: URI, map: Map<string, AgentDefinition>): Promise<void> {
		try {
			const stat = await this.fileService.resolve(uri);
			if (!stat.children) {
				return;
			}
			for (const child of stat.children) {
				if (child.isDirectory || !child.resource.path.endsWith('.md')) {
					continue;
				}
				try {
					const content = await this.fileService.readFile(child.resource);
					const agent = parseAgentMarkdown(content.value.toString());
					if (agent) {
						map.set(agent.name, { ...agent, sourcePath: child.resource.path });
					}
				} catch {
					// Unreadable file — skip
				}
			}
		} catch {
			// Directory doesn't exist — skip
		}
	}

	private async loadSkillsDir(uri: URI, map: Map<string, SkillDefinition>): Promise<void> {
		try {
			const stat = await this.fileService.resolve(uri);
			if (!stat.children) {
				return;
			}
			for (const child of stat.children) {
				if (child.isDirectory || !child.resource.path.endsWith('.md')) {
					continue;
				}
				try {
					const content = await this.fileService.readFile(child.resource);
					const skill = parseSkillMarkdown(content.value.toString());
					if (skill) {
						map.set(skill.name, { ...skill, sourcePath: child.resource.path });
					}
				} catch {
					// Unreadable file — skip
				}
			}
		} catch {
			// Directory doesn't exist — skip
		}
	}

	private expandHome(path: string, homePath: string): string {
		if (path.startsWith('~/')) {
			return homePath + path.slice(1);
		}
		return path;
	}
}
