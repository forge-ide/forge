/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { createDecorator } from '../../../../platform/instantiation/common/instantiation.js';
import { CancellationToken } from '../../../../base/common/cancellation.js';
import { URI } from '../../../../base/common/uri.js';
import type { ForgeContextItem } from './forgeContextTypes.js';

export const IForgeGitDiffService = createDecorator<IForgeGitDiffService>('forgeGitDiffService');

export interface IForgeGitDiffService {
	readonly _serviceBrand: undefined;
	resolveGitDiff(workspaceRoot: URI, maxChars?: number, token?: CancellationToken): Promise<ForgeContextItem>;
}
