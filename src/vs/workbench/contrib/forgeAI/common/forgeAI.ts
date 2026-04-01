/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { RawContextKey } from '../../../../platform/contextkey/common/contextkey.js';

/**
 * Forge AI viewlet (view container) identifier.
 */
export const FORGE_AI_VIEWLET_ID = 'workbench.view.forgeAI';

/**
 * Forge AI workspace view identifier.
 */
export const FORGE_AI_WORKSPACE_VIEW_ID = 'workbench.forgeAI.workspaceView';

/**
 * Context key: true when at least one Forge workspace has been saved.
 */
export const FORGE_AI_HAS_WORKSPACES = new RawContextKey<boolean>('forgeAI.hasWorkspaces', false);
