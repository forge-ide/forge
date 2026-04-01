/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { Event } from '../../../../base/common/event.js';
import { createDecorator } from '../../../../platform/instantiation/common/instantiation.js';
import type { ForgeWorkspaceConfig } from './forgeWorkspaceTypes.js';

export const IForgeWorkspaceService = createDecorator<IForgeWorkspaceService>('forgeWorkspaceService');

export interface IForgeWorkspaceService {
	readonly _serviceBrand: undefined;
	readonly onDidChangeActiveWorkspace: Event<ForgeWorkspaceConfig | undefined>;
	readonly onDidChangeWorkspaces: Event<void>;
	getWorkspaces(): ForgeWorkspaceConfig[];
	getActiveWorkspace(): ForgeWorkspaceConfig | undefined;
	createWorkspace(name: string): Promise<ForgeWorkspaceConfig>;
	saveActiveWorkspace(): Promise<void>;
	switchWorkspace(id: string): Promise<void>;
	deleteWorkspace(id: string): Promise<void>;
	renameWorkspace(id: string, newName: string): Promise<void>;
}
