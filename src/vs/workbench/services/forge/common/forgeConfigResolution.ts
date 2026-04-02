import { createDecorator } from '../../../../platform/instantiation/common/instantiation.js';
import { Event } from '../../../../base/common/event.js';
import { ResolvedConfig } from './forgeConfigResolutionTypes.js';

export const IForgeConfigResolutionService = createDecorator<IForgeConfigResolutionService>('forgeConfigResolutionService');

export interface IForgeConfigResolutionService {
	readonly _serviceBrand: undefined;

	readonly onDidChangeResolved: Event<ResolvedConfig>;

	/**
	 * Discover and merge all config sources for the given workspace root.
	 * Resolution order: global (~/) → configPaths → project (./) → disabled filter.
	 */
	resolve(workspaceRoot: string): Promise<ResolvedConfig>;

	/** Get the last resolved config without re-reading files. */
	getCached(): ResolvedConfig | undefined;

	/** Toggle an MCP server's disabled state. Persists to forge.json. */
	setMcpServerDisabled(serverName: string, disabled: boolean): Promise<void>;

	/** Toggle an agent's disabled state. Persists to forge.json. */
	setAgentDisabled(agentName: string, disabled: boolean): Promise<void>;
}
