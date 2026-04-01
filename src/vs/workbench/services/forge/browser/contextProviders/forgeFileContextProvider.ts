/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { Disposable } from '../../../../../base/common/lifecycle.js';
import { URI } from '../../../../../base/common/uri.js';
import { CancellationToken } from '../../../../../base/common/cancellation.js';
import { IFileService } from '../../../../../platform/files/common/files.js';
import { ILogService } from '../../../../../platform/log/common/log.js';
import { ForgeContextItem, ForgeContextType } from '../../common/forgeContextTypes.js';

export class ForgeFileContextProvider extends Disposable {

	constructor(
		@IFileService private readonly fileService: IFileService,
		@ILogService private readonly logService: ILogService,
	) {
		super();
	}

	async resolveFile(uri: URI, maxChars: number = 32000, token?: CancellationToken): Promise<ForgeContextItem> {
		const fileName = uri.path.split('/').pop() || uri.path;

		try {
			const fileContent = await this.fileService.readFile(uri);
			if (token?.isCancellationRequested) {
				return {
					type: ForgeContextType.File,
					label: fileName,
					detail: uri.path,
					content: '',
					tokenEstimate: 0,
					uri,
				};
			}

			let text = fileContent.value.toString();
			if (text.length > maxChars) {
				text = this.truncateFromMiddle(text, maxChars);
			}

			return {
				type: ForgeContextType.File,
				label: fileName,
				detail: uri.path,
				content: text,
				tokenEstimate: Math.ceil(text.length / 4),
				uri,
			};
		} catch (error) {
			this.logService.warn(`[ForgeFileContextProvider] Failed to read file: ${uri.toString()}`, error);
			const errorMessage = error instanceof Error ? error.message : 'Unknown error reading file';
			return {
				type: ForgeContextType.File,
				label: fileName,
				detail: uri.path,
				content: `[Error reading file: ${errorMessage}]`,
				tokenEstimate: Math.ceil(errorMessage.length / 4),
				uri,
			};
		}
	}

	private truncateFromMiddle(content: string, maxChars: number): string {
		if (content.length <= maxChars) {
			return content;
		}

		const halfLen = Math.floor(maxChars / 2);
		const truncatedCount = content.length - maxChars;
		const head = content.substring(0, halfLen);
		const tail = content.substring(content.length - halfLen);

		return `${head}\n\n[...truncated ${truncatedCount} characters from middle...]\n\n${tail}`;
	}
}
