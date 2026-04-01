/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { execFile } from 'child_process';
import { promisify } from 'util';
import { Disposable } from '../../../../../base/common/lifecycle.js';
import { URI } from '../../../../../base/common/uri.js';
import { CancellationToken } from '../../../../../base/common/cancellation.js';
import { ILogService } from '../../../../../platform/log/common/log.js';
import { InstantiationType, registerSingleton } from '../../../../../platform/instantiation/common/extensions.js';
import { ForgeContextItem, ForgeContextType } from '../../common/forgeContextTypes.js';
import { IForgeGitDiffService } from '../../common/forgeGitDiffService.js';

const execFileAsync = promisify(execFile);

export class ForgeGitDiffContextProvider extends Disposable implements IForgeGitDiffService {

	declare readonly _serviceBrand: undefined;

	constructor(
		@ILogService private readonly logService: ILogService,
	) {
		super();
	}

	async resolveGitDiff(workspaceRoot: URI, maxChars: number = 32000, token?: CancellationToken): Promise<ForgeContextItem> {
		const fsPath = workspaceRoot.fsPath;

		if (token?.isCancellationRequested) {
			return this.createEmptyItem();
		}

		let stdout: string;
		try {
			stdout = await this.execGitDiff(fsPath);
		} catch (error) {
			this.logService.warn('[ForgeGitDiffContextProvider] git diff failed', error);
			return this.createItem('Git diff unavailable: failed to run git diff.', 'Git Diff (error)');
		}

		if (token?.isCancellationRequested) {
			return this.createEmptyItem();
		}

		if (!stdout || stdout.trim().length === 0) {
			return this.createItem('No changes detected.', 'Git Diff (HEAD)');
		}

		let content = stdout;
		if (content.length > maxChars) {
			const halfLen = Math.floor(maxChars / 2);
			const head = content.substring(0, halfLen);
			const tail = content.substring(content.length - halfLen);
			const truncatedCount = content.length - maxChars;
			content = `${head}\n\n[...truncated ${truncatedCount} characters from middle...]\n\n${tail}`;
		}

		return this.createItem(content, 'Git Diff (HEAD)');
	}

	protected async execGitDiff(cwd: string): Promise<string> {
		const result = await execFileAsync('git', ['diff', 'HEAD'], { cwd, timeout: 5000 });
		return result.stdout;
	}

	private createEmptyItem(): ForgeContextItem {
		return this.createItem('', 'Git Diff (empty)');
	}

	private createItem(content: string, label: string): ForgeContextItem {
		return {
			type: ForgeContextType.GitDiff,
			label,
			content,
			tokenEstimate: Math.ceil(content.length / 4),
		};
	}
}

registerSingleton(IForgeGitDiffService, ForgeGitDiffContextProvider, InstantiationType.Delayed);
