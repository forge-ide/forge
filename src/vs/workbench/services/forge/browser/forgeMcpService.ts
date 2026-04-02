import { Disposable } from '../../../../base/common/lifecycle.js';
import { Emitter, Event } from '../../../../base/common/event.js';
import { ILogService } from '../../../../platform/log/common/log.js';
import { AIToolDefinition } from '../../../../platform/ai/common/aiProvider.js';
import { createDecorator } from '../../../../platform/instantiation/common/instantiation.js';
import { IForgeConfigResolutionService } from '../common/forgeConfigResolution.js';
import { ForgeMcpServerStatus, ForgeMcpToolResultRecord, createToolResultRecord } from '../common/forgeMcpTypes.js';
import { IForgeMcpService, ForgeMcpServerStatusEntry } from '../common/forgeMcpService.js';

/**
 * Minimal observable-like accessor. VS Code's IObservable requires a reader context;
 * we only need `.get()` for synchronous reads outside of reactive contexts.
 */
interface IReadable<T> {
	get(): T;
}

/** Minimal shape of an MCP tool sufficient for Forge's bridge. */
interface IMcpToolMin {
	readonly definition: {
		readonly name: string;
		readonly description?: string;
		readonly inputSchema: Record<string, unknown>;
	};
	call(params: Record<string, unknown>): Promise<{
		content: Array<{ type: string; text?: string } & Record<string, unknown>>;
		isError?: boolean;
	}>;
}

/** Minimal shape of an MCP server sufficient for Forge's bridge. */
interface IMcpServerMin {
	readonly definition: { readonly id: string; readonly label: string };
	readonly connectionState: IReadable<{ readonly state: number }>;
	readonly tools: IReadable<readonly IMcpToolMin[]>;
}

/** Minimal shape of the VS Code IMcpService sufficient for Forge's bridge. */
export interface IForgeMcpBridgeHost {
	readonly _serviceBrand: undefined;
	readonly servers: IReadable<readonly IMcpServerMin[]>;
}

export const IForgeMcpBridgeHost = createDecorator<IForgeMcpBridgeHost>('forgeMcpBridgeHost');

/** McpConnectionState.Kind values (must match vs/workbench/contrib/mcp/common/mcpTypes.ts) */
const enum McpKind {
	Stopped = 0,
	Starting = 1,
	Running = 2,
	Error = 3,
}

export class ForgeMcpService extends Disposable implements IForgeMcpService {
	declare readonly _serviceBrand: undefined;

	private readonly _onDidChangeTools = this._register(new Emitter<void>());
	readonly onDidChangeTools: Event<void> = this._onDidChangeTools.event;

	private readonly _onDidChangeServerStatus = this._register(new Emitter<ForgeMcpServerStatusEntry>());
	readonly onDidChangeServerStatus: Event<ForgeMcpServerStatusEntry> = this._onDidChangeServerStatus.event;

	constructor(
		@IForgeMcpBridgeHost private readonly mcpService: IForgeMcpBridgeHost,
		@IForgeConfigResolutionService private readonly configResolution: IForgeConfigResolutionService,
		@ILogService private readonly logService: ILogService,
	) {
		super();

		this._register(configResolution.onDidChangeResolved(() => {
			this._onDidChangeTools.fire();
		}));
	}

	async listTools(): Promise<AIToolDefinition[]> {
		const tools: AIToolDefinition[] = [];
		const disabledServers = this._getDisabledServerNames();

		for (const server of this.mcpService.servers.get()) {
			const serverName = server.definition.label;
			if (disabledServers.has(serverName)) {
				continue;
			}

			const connectionState = server.connectionState.get();
			if (connectionState.state !== McpKind.Running) {
				continue;
			}

			for (const tool of server.tools.get()) {
				tools.push({
					name: tool.definition.name,
					description: tool.definition.description ?? '',
					inputSchema: tool.definition.inputSchema as Record<string, unknown>,
				});
			}
		}

		return tools;
	}

	async callTool(toolName: string, input: Record<string, unknown>): Promise<ForgeMcpToolResultRecord> {
		const disabledServers = this._getDisabledServerNames();

		for (const server of this.mcpService.servers.get()) {
			const serverName = server.definition.label;
			if (disabledServers.has(serverName)) {
				continue;
			}

			const tool = server.tools.get().find(t => t.definition.name === toolName);
			if (!tool) {
				continue;
			}

			const callId = `${toolName}-${Date.now()}`;
			try {
				const result = await tool.call(input);
				const content = result.content
					.map(c => {
						if (c.type === 'text') {
							return c.text ?? '';
						}
						return JSON.stringify(c);
					})
					.join('\n');
				return createToolResultRecord(callId, content, result.isError ?? false);
			} catch (err) {
				this.logService.error(`[ForgeMcpService] Tool call failed: ${toolName}`, err);
				const message = err instanceof Error ? err.message : String(err);
				return createToolResultRecord(callId, message, true);
			}
		}

		throw new Error(`Tool "${toolName}" not found`);
	}

	getServerStatuses(): ForgeMcpServerStatusEntry[] {
		const disabledServers = this._getDisabledServerNames();
		const statuses: ForgeMcpServerStatusEntry[] = [];

		for (const server of this.mcpService.servers.get()) {
			const name = server.definition.label;
			const connectionState = server.connectionState.get();
			const status = this._mapConnectionState(connectionState);
			const toolCount = connectionState.state === McpKind.Running ? server.tools.get().length : 0;

			statuses.push({
				name,
				status,
				toolCount,
				disabled: disabledServers.has(name),
			});
		}

		return statuses;
	}

	isServerDisabled(serverName: string): boolean {
		return this._getDisabledServerNames().has(serverName);
	}

	async toggleServerDisabled(serverName: string, disabled: boolean): Promise<void> {
		await this.configResolution.setMcpServerDisabled(serverName, disabled);
	}

	private _getDisabledServerNames(): Set<string> {
		const cached = this.configResolution.getCached();
		if (!cached) {
			return new Set();
		}
		return new Set(cached.disabled.mcpServers);
	}

	private _mapConnectionState(state: { readonly state: number }): ForgeMcpServerStatus {
		switch (state.state) {
			case McpKind.Running:
				return ForgeMcpServerStatus.Connected;
			case McpKind.Starting:
				return ForgeMcpServerStatus.Connecting;
			case McpKind.Error:
				return ForgeMcpServerStatus.Error;
			case McpKind.Stopped:
			default:
				return ForgeMcpServerStatus.Disconnected;
		}
	}
}
