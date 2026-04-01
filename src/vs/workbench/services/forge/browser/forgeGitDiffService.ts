/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { Disposable } from '../../../../base/common/lifecycle.js';
import { URI } from '../../../../base/common/uri.js';
import { CancellationToken } from '../../../../base/common/cancellation.js';
import { InstantiationType, registerSingleton } from '../../../../platform/instantiation/common/extensions.js';
import { ForgeContextItem, ForgeContextType } from '../common/forgeContextTypes.js';
import { IForgeGitDiffService } from '../common/forgeGitDiffService.js';

/**
 * Browser-safe stub for IForgeGitDiffService.
 * Git diff requires Node.js child_process which is unavailable in the renderer.
 * The desktop build overrides this with the node implementation via IPC.
 */
export class ForgeGitDiffService extends Disposable implements IForgeGitDiffService {

	declare readonly _serviceBrand: undefined;

	async resolveGitDiff(_workspaceRoot: URI, _maxChars?: number, _token?: CancellationToken): Promise<ForgeContextItem> {
		return {
			type: ForgeContextType.GitDiff,
			label: 'Git Diff (unavailable)',
			content: 'Git diff is not available in this context.',
			tokenEstimate: 0,
		};
	}
}

registerSingleton(IForgeGitDiffService, ForgeGitDiffService, InstantiationType.Delayed);
