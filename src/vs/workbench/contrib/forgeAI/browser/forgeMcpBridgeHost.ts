/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { Disposable } from '../../../../base/common/lifecycle.js';
import { IMcpService } from '../../mcp/common/mcpTypes.js';
import { IForgeMcpBridgeHost } from '../../../services/forge/browser/forgeMcpService.js';

/**
 * Bridges VS Code's IMcpService to the minimal IForgeMcpBridgeHost shape
 * that ForgeMcpService depends on. Lives in contrib so it can import from
 * contrib/mcp without violating the services/ layering rules.
 */
export class ForgeMcpBridgeHost extends Disposable implements IForgeMcpBridgeHost {
	declare readonly _serviceBrand: undefined;

	readonly servers = {
		get: () => this._mcpService.servers.get().map(server => ({
			definition: {
				id: server.definition.id,
				label: server.definition.label,
			},
			connectionState: {
				get: () => ({ state: server.connectionState.get().state }),
			},
			tools: {
				get: () => server.tools.get().map(tool => ({
					definition: {
						name: tool.definition.name,
						description: tool.definition.description,
						inputSchema: tool.definition.inputSchema as Record<string, unknown>,
					},
					call: (params: Record<string, unknown>) =>
						tool.call(params) as unknown as Promise<{
							content: Array<{ type: string; text?: string } & Record<string, unknown>>;
							isError?: boolean;
						}>,
				})),
			},
		})),
	};

	constructor(@IMcpService private readonly _mcpService: IMcpService) {
		super();
	}
}
