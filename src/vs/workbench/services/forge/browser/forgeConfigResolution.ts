import { VSBuffer } from '../../../../base/common/buffer.js';
import { Disposable } from '../../../../base/common/lifecycle.js';
import { Emitter, Event } from '../../../../base/common/event.js';
import { IFileService } from '../../../../platform/files/common/files.js';
import { URI } from '../../../../base/common/uri.js';
import { ILogService } from '../../../../platform/log/common/log.js';
import { IEnvironmentService } from '../../../../platform/environment/common/environment.js';
import { IForgeConfigResolutionService } from '../common/forgeConfigResolution.js';
import {
	ResolvedConfig,
	McpServerEntry,
	AgentDefinition,
	SkillDefinition,
	DisabledConfig,
	ConfigPaths,
	parseMcpJson,
	parseAgentMarkdown,
} from '../common/forgeConfigResolutionTypes.js';
import { parseSkillMarkdown } from '../common/forgeSkillTypes.js';
import { IPathService } from '../../path/common/pathService.js';

export class ForgeConfigResolutionService
	extends Disposable
	implements IForgeConfigResolutionService {
	declare readonly _serviceBrand: undefined;

	private readonly _onDidChangeResolved = this._register(
		new Emitter<ResolvedConfig>(),
	);
	readonly onDidChangeResolved: Event<ResolvedConfig> =
		this._onDidChangeResolved.event;

	private readonly _globalForgeJsonUri: URI;

	private _cached: ResolvedConfig | undefined;
	private _lastWorkspaceRoot: string | undefined;
	private _debounceTimer: ReturnType<typeof setTimeout> | undefined;
	private _watchersInitialized = false;

	constructor(
		@IFileService private readonly fileService: IFileService,
		@IEnvironmentService environmentService: IEnvironmentService,
		@IPathService private readonly pathService: IPathService,
		@ILogService private readonly logService: ILogService,
	) {
		super();
		this._globalForgeJsonUri = URI.joinPath(
			environmentService.userRoamingDataHome,
			'forge',
			'forge.json',
		);
	}

	getCached(): ResolvedConfig | undefined {
		return this._cached;
	}

	async resolve(workspaceRoot: string): Promise<ResolvedConfig> {
		const { configPaths, disabled: rawDisabled } = await this.readDiscoveryConfig(workspaceRoot);
		const disabled: DisabledConfig = rawDisabled ?? { mcpServers: [], agents: [] };
		const homeUri = await this.pathService.userHome();
		const homePath = homeUri.path;

		// --- Collect MCP servers ---
		const serverMap = new Map<string, McpServerEntry>();

		// 1. Global ~/.mcp.json
		await this.loadMcpJson(
			URI.joinPath(URI.file(homePath), '.mcp.json'),
			serverMap,
		);

		// 2. configPaths.mcp entries
		if (configPaths?.mcp) {
			for (const dir of configPaths.mcp) {
				const resolved = this.expandHome(dir, homePath);
				await this.loadMcpJson(
					URI.joinPath(URI.file(resolved), '.mcp.json'),
					serverMap,
				);
			}
		}

		// 3. Project .mcp.json (wins on conflict)
		await this.loadMcpJson(
			URI.joinPath(URI.file(workspaceRoot), '.mcp.json'),
			serverMap,
		);

		// --- Collect agents ---
		const agentMap = new Map<string, AgentDefinition>();

		// 1. Global ~/.agents/
		await this.loadAgentsDir(
			URI.joinPath(URI.file(homePath), '.agents'),
			agentMap,
		);

		// 2. configPaths.agents entries
		if (configPaths?.agents) {
			for (const dir of configPaths.agents) {
				const resolved = this.expandHome(dir, homePath);
				await this.loadAgentsDir(URI.file(resolved), agentMap);
			}
		}

		// 3. Project .agents/
		await this.loadAgentsDir(
			URI.joinPath(URI.file(workspaceRoot), '.agents'),
			agentMap,
		);

		// --- Collect skills ---
		const skillMap = new Map<string, SkillDefinition>();

		// 1. Global ~/.skills/
		await this.loadSkillsDir(
			URI.joinPath(URI.file(homePath), '.skills'),
			skillMap,
		);

		// 2. configPaths.skills entries
		if (configPaths?.skills) {
			for (const dir of configPaths.skills) {
				const resolved = this.expandHome(dir, homePath);
				await this.loadSkillsDir(URI.file(resolved), skillMap);
			}
		}

		// 3. Project .skills/
		await this.loadSkillsDir(
			URI.joinPath(URI.file(workspaceRoot), '.skills'),
			skillMap,
		);

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
		this._lastWorkspaceRoot = workspaceRoot;

		if (!this._watchersInitialized) {
			this._watchersInitialized = true;
			this._initFileWatchers(workspaceRoot, homePath);
		}

		this._onDidChangeResolved.fire(resolvedConfig);
		return resolvedConfig;
	}

	async setMcpServerDisabled(
		serverName: string,
		disabled: boolean,
	): Promise<void> {
		if (!this._lastWorkspaceRoot) {
			this.logService.warn('[ForgeConfigResolution] setMcpServerDisabled called before resolve() — writing to global forge.json');
		}
		const { disabled: current, sourceUri } = await this.readDiscoveryConfig(this._lastWorkspaceRoot ?? '');
		const currentDisabled = current ?? { mcpServers: [], agents: [] };
		const mcpSet = new Set(currentDisabled.mcpServers);

		if (disabled) {
			mcpSet.add(serverName);
		} else {
			mcpSet.delete(serverName);
		}

		await this.writeDiscoveryConfig(
			{ disabled: { mcpServers: Array.from(mcpSet), agents: currentDisabled.agents } },
			sourceUri,
		);
	}

	async setAgentDisabled(agentName: string, disabled: boolean): Promise<void> {
		if (!this._lastWorkspaceRoot) {
			this.logService.warn('[ForgeConfigResolution] setAgentDisabled called before resolve() — writing to global forge.json');
		}
		const { disabled: current, sourceUri } = await this.readDiscoveryConfig(this._lastWorkspaceRoot ?? '');
		const currentDisabled = current ?? { mcpServers: [], agents: [] };
		const agentSet = new Set(currentDisabled.agents);

		if (disabled) {
			agentSet.add(agentName);
		} else {
			agentSet.delete(agentName);
		}

		await this.writeDiscoveryConfig(
			{ disabled: { mcpServers: currentDisabled.mcpServers, agents: Array.from(agentSet) } },
			sourceUri,
		);
	}

	// --- Discovery config (forge.json) ---

	private async readDiscoveryConfig(workspaceRoot: string): Promise<{ configPaths?: ConfigPaths; disabled?: DisabledConfig; sourceUri: URI }> {
		const candidates = [
			URI.joinPath(URI.file(workspaceRoot), 'forge.json'),
			this._globalForgeJsonUri,
		];

		for (const uri of candidates) {
			try {
				const content = await this.fileService.readFile(uri);
				const parsed = JSON.parse(content.value.toString()) as { configPaths?: ConfigPaths; disabled?: DisabledConfig };
				if (parsed.configPaths !== undefined || parsed.disabled !== undefined) {
					return {
						configPaths: parsed.configPaths,
						disabled: parsed.disabled,
						sourceUri: uri,
					};
				}
			} catch {
				// File absent or unreadable — try next candidate
			}
		}

		return { sourceUri: this._globalForgeJsonUri };
	}

	private async writeDiscoveryConfig(partial: { configPaths?: ConfigPaths; disabled?: DisabledConfig }, targetUri: URI): Promise<void> {
		let existing: Record<string, unknown> = {};
		try {
			const content = await this.fileService.readFile(targetUri);
			existing = JSON.parse(content.value.toString()) as Record<string, unknown>;
		} catch {
			// File doesn't exist yet — will be created
		}

		const updated = { ...existing, ...partial };
		const serialized = JSON.stringify(updated, undefined, '\t');
		await this.fileService.writeFile(targetUri, VSBuffer.fromString(serialized));
	}

	// --- File watching ---

	private _initFileWatchers(workspaceRoot: string, homePath: string): void {
		const watchTargets = [
			URI.joinPath(URI.file(homePath), '.mcp.json'),
			URI.joinPath(URI.file(homePath), '.agents'),
			URI.joinPath(URI.file(homePath), '.skills'),
			URI.joinPath(URI.file(workspaceRoot), '.mcp.json'),
			URI.joinPath(URI.file(workspaceRoot), '.agents'),
			URI.joinPath(URI.file(workspaceRoot), '.skills'),
		];

		for (const uri of watchTargets) {
			this._register(
				this.fileService.watch(uri, { recursive: false, excludes: [] }),
			);
		}

		this._register({
			dispose: () => {
				if (this._debounceTimer !== undefined) {
					clearTimeout(this._debounceTimer);
					this._debounceTimer = undefined;
				}
			},
		});

		this._register(
			this.fileService.onDidFilesChange(() => {
				if (this._debounceTimer !== undefined) {
					clearTimeout(this._debounceTimer);
				}
				this._debounceTimer = setTimeout(() => {
					this._debounceTimer = undefined;
					if (this._lastWorkspaceRoot) {
						this.resolve(this._lastWorkspaceRoot).catch((err) => {
							this.logService.error(
								'[ForgeConfigResolution] Re-resolve after file change failed:',
								err,
							);
						});
					}
				}, 300);
			}),
		);
	}

	// --- Private helpers ---

	private async loadMcpJson(
		uri: URI,
		map: Map<string, McpServerEntry>,
	): Promise<void> {
		try {
			const content = await this.fileService.readFile(uri);
			const raw = JSON.parse(content.value.toString());
			const result = parseMcpJson(raw);
			if (result.valid) {
				for (const server of result.servers) {
					map.set(server.name, server);
				}
			}
		} catch (e) {
			this.logService.debug(
				`[ForgeConfigResolution] Failed to load ${uri.toString()}: ${e}`,
			);
		}
	}

	private async loadAgentsDir(
		uri: URI,
		map: Map<string, AgentDefinition>,
	): Promise<void> {
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
				} catch (e) {
					this.logService.debug(
						`[ForgeConfigResolution] Failed to load agent ${child.resource.toString()}: ${e}`,
					);
				}
			}
		} catch (e) {
			this.logService.debug(
				`[ForgeConfigResolution] Agents dir not found ${uri.toString()}: ${e}`,
			);
		}
	}

	private async loadSkillsDir(
		uri: URI,
		map: Map<string, SkillDefinition>,
	): Promise<void> {
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
				} catch (e) {
					this.logService.debug(
						`[ForgeConfigResolution] Failed to load skill ${child.resource.toString()}: ${e}`,
					);
				}
			}
		} catch (e) {
			this.logService.debug(
				`[ForgeConfigResolution] Skills dir not found ${uri.toString()}: ${e}`,
			);
		}
	}

	private expandHome(path: string, homePath: string): string {
		if (path.startsWith('~/')) {
			return homePath + path.slice(1);
		}
		return path;
	}
}
