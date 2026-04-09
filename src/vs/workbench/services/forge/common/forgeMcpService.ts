import { createDecorator } from '../../../../platform/instantiation/common/instantiation.js';
import { Event } from '../../../../base/common/event.js';
import { AIToolDefinition } from '../../../../platform/ai/common/aiProvider.js';
import { ForgeMcpServerStatus, ForgeMcpToolResultRecord } from './forgeMcpTypes.js';

export const IForgeMcpService = createDecorator<IForgeMcpService>('forgeMcpService');

export interface ForgeMcpServerStatusEntry {
	readonly name: string;
	readonly status: ForgeMcpServerStatus;
	readonly toolCount: number;
	readonly disabled: boolean;
	readonly transport: 'local' | 'remote';
}

export interface IForgeMcpService {
	readonly _serviceBrand: undefined;

	readonly onDidChangeTools: Event<void>;
	readonly onDidChangeServerStatus: Event<ForgeMcpServerStatusEntry>;

	listTools(): Promise<AIToolDefinition[]>;
	callTool(toolName: string, input: Record<string, unknown>): Promise<ForgeMcpToolResultRecord>;
	getServerStatuses(): ForgeMcpServerStatusEntry[];

	/** Check if a server is in the disabled list. */
	isServerDisabled(serverName: string): boolean;

	/** Toggle a server's disabled state. Persists to forge.json and re-resolves config. */
	toggleServerDisabled(serverName: string, disabled: boolean): Promise<void>;
}
