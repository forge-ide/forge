/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { Emitter } from '../../../../base/common/event.js';
import { Disposable } from '../../../../base/common/lifecycle.js';
import { URI } from '../../../../base/common/uri.js';
import { CancellationToken } from '../../../../base/common/cancellation.js';
import { isCodeEditor } from '../../../../editor/browser/editorBrowser.js';
import { ILogService } from '../../../../platform/log/common/log.js';
import { IQuickInputService, IQuickPickItem, IQuickPickSeparator } from '../../../../platform/quickinput/common/quickInput.js';
import { InstantiationType, registerSingleton } from '../../../../platform/instantiation/common/extensions.js';
import { IInstantiationService } from '../../../../platform/instantiation/common/instantiation.js';
import { IWorkspaceContextService } from '../../../../platform/workspace/common/workspace.js';
import { IEditorService } from '../../../services/editor/common/editorService.js';
import { IEditorGroupsService } from '../../../services/editor/common/editorGroupsService.js';
import { IForgeLayoutService, type PanePosition } from '../common/forgeLayoutService.js';
import { IForgeContextService } from '../common/forgeContextService.js';
import { IForgeGitDiffService } from '../common/forgeGitDiffService.js';
import { ForgeContextChip, ForgeContextItem, ForgeContextType, IForgeContextBudget, formatContextItem } from '../common/forgeContextTypes.js';
import { ForgeFileContextProvider } from './contextProviders/forgeFileContextProvider.js';
import { ForgePaneHistoryContextProvider } from './contextProviders/forgePaneHistoryContextProvider.js';

/** Priority order for context types when fitting into the token budget. Lower = higher priority. */
const CONTEXT_PRIORITY: Record<ForgeContextType, number> = {
	activeEditor: 0,
	selection: 1,
	file: 2,
	gitDiff: 3,
	symbol: 4,
	paneHistory: 5,
};

function paneKey(position: PanePosition | undefined): string {
	return position ?? 'default';
}

function estimateTokens(content: string): number {
	return Math.ceil(content.length / 4);
}

export class ForgeContextService extends Disposable implements IForgeContextService {

	declare readonly _serviceBrand: undefined;

	private readonly _onDidChangeContext = this._register(new Emitter<PanePosition | undefined>());
	readonly onDidChangeContext = this._onDidChangeContext.event;

	private readonly _chipStore = new Map<string, ForgeContextChip[]>();
	private readonly fileContextProvider: ForgeFileContextProvider;
	private readonly paneHistoryProvider: ForgePaneHistoryContextProvider;

	constructor(
		@IEditorService private readonly editorService: IEditorService,
		@IEditorGroupsService private readonly editorGroupsService: IEditorGroupsService,
		@IQuickInputService private readonly quickInputService: IQuickInputService,
		@IForgeLayoutService private readonly layoutService: IForgeLayoutService,
		@ILogService private readonly logService: ILogService,
		@IInstantiationService private readonly instantiationService: IInstantiationService,
		@IForgeGitDiffService private readonly gitDiffService: IForgeGitDiffService,
		@IWorkspaceContextService private readonly workspaceContextService: IWorkspaceContextService,
	) {
		super();

		this.fileContextProvider = this._register(this.instantiationService.createInstance(ForgeFileContextProvider));
		this.paneHistoryProvider = this._register(this.instantiationService.createInstance(ForgePaneHistoryContextProvider));

		this._register(this.layoutService.onDidChangeLayout(() => {
			this.pruneStaleEntries();
		}));

		this._register(this.editorGroupsService.onDidRemoveGroup(() => {
			this.pruneStaleEntries();
		}));
	}

	getContextChips(panePosition: PanePosition | undefined): ForgeContextChip[] {
		return this._chipStore.get(paneKey(panePosition)) ?? [];
	}

	addContextChip(panePosition: PanePosition | undefined, item: ForgeContextItem, automatic: boolean = false): void {
		const key = paneKey(panePosition);
		const chips = this._chipStore.get(key) ?? [];

		// Deduplicate by type + label
		const existing = chips.findIndex(c => c.item.type === item.type && c.item.label === item.label);
		if (existing !== -1) {
			chips[existing] = { item, automatic };
		} else {
			chips.push({ item, automatic });
		}

		this._chipStore.set(key, chips);
		this._onDidChangeContext.fire(panePosition);
		this.logService.debug(`[ForgeContextService] Added context chip: ${item.type} "${item.label}" (${item.tokenEstimate} tokens)`);
	}

	removeContextChip(panePosition: PanePosition | undefined, item: ForgeContextItem): void {
		const key = paneKey(panePosition);
		const chips = this._chipStore.get(key);
		if (!chips) {
			return;
		}

		const index = chips.findIndex(c => c.item.type === item.type && c.item.label === item.label);
		if (index !== -1) {
			chips.splice(index, 1);
			this._chipStore.set(key, chips);
			this._onDidChangeContext.fire(panePosition);
		}
	}

	clearContext(panePosition: PanePosition | undefined): void {
		const key = paneKey(panePosition);
		this._chipStore.delete(key);
		this._onDidChangeContext.fire(panePosition);
	}

	async resolveContextPrompt(
		panePosition: PanePosition | undefined,
		maxTokens: number,
		token?: CancellationToken,
	): Promise<IForgeContextBudget> {
		const chips = this.getContextChips(panePosition);
		if (chips.length === 0) {
			return { maxTokens, usedTokens: 0, items: [], droppedCount: 0 };
		}

		// Sort by priority (lower number = higher priority)
		const sorted = [...chips].sort(
			(a, b) => (CONTEXT_PRIORITY[a.item.type] ?? 99) - (CONTEXT_PRIORITY[b.item.type] ?? 99)
		);

		// Resolve file content for chips that have a URI but minimal content
		const resolved = await this.resolveChipContent(sorted, token);

		const included: ForgeContextChip[] = [];
		let usedTokens = 0;
		let droppedCount = 0;

		for (const chip of resolved) {
			if (token?.isCancellationRequested) {
				break;
			}
			const formatted = formatContextItem(chip.item);
			const tokenCost = estimateTokens(formatted);

			if (usedTokens + tokenCost <= maxTokens) {
				usedTokens += tokenCost;
				included.push(chip);
			} else {
				droppedCount++;
			}
		}

		return { maxTokens, usedTokens, items: included, droppedCount };
	}

	/**
	 * For file/activeEditor chips that have a URI but minimal content,
	 * resolve full file content via the file context provider.
	 */
	private async resolveChipContent(chips: ForgeContextChip[], token?: CancellationToken): Promise<ForgeContextChip[]> {
		const result: ForgeContextChip[] = [];

		for (const chip of chips) {
			if (token?.isCancellationRequested) {
				result.push(chip);
				continue;
			}

			const needsResolution =
				(chip.item.type === ForgeContextType.File || chip.item.type === ForgeContextType.ActiveEditor) &&
				chip.item.uri &&
				chip.item.content.length < 100;

			if (needsResolution) {
				const resolved = await this.fileContextProvider.resolveFile(chip.item.uri!, 32000, token);
				result.push({
					item: {
						...resolved,
						type: chip.item.type,
						detail: chip.item.detail ?? resolved.detail,
						sourcePanePosition: chip.item.sourcePanePosition,
					},
					automatic: chip.automatic,
				});
			} else {
				result.push(chip);
			}
		}

		return result;
	}

	async showContextPicker(panePosition: PanePosition | undefined): Promise<ForgeContextItem[]> {
		const picks: (IQuickPickItem | IQuickPickSeparator)[] = [];

		// Section: Active selection (shown first for immediate relevance)
		const activeControl = this.editorService.activeTextEditorControl;
		if (isCodeEditor(activeControl)) {
			const selection = activeControl.getSelection();
			if (selection && !selection.isEmpty()) {
				picks.push({ type: 'separator', label: 'SELECTION' });
				picks.push({
					label: '$(selection) Current Selection',
					description: `Lines ${selection.startLineNumber}-${selection.endLineNumber}`,
					detail: 'Selection',
					id: 'selection:current',
				});
			}
		}

		// Section: FILES — open editors
		const editors = this.editorService.editors;
		const seenUris = new Set<string>();
		const fileItems: IQuickPickItem[] = [];
		for (const editor of editors) {
			const uri = editor.resource;
			if (uri && !seenUris.has(uri.toString())) {
				seenUris.add(uri.toString());
				fileItems.push({
					label: `$(file) ${uri.path.split('/').pop() ?? uri.path}`,
					description: uri.path,
					detail: 'File',
					id: `file:${uri.toString()}`,
				});
			}
		}
		if (fileItems.length > 0) {
			picks.push({ type: 'separator', label: 'FILES' });
			picks.push(...fileItems);
		}

		// Section: Pane histories (cross-pane context)
		const paneHistories = this.paneHistoryProvider.getAvailablePaneHistories(panePosition);
		if (paneHistories.length > 0) {
			picks.push({ type: 'separator', label: 'PANE HISTORY' });
			for (const paneItem of paneHistories) {
				picks.push({
					label: `$(comment-discussion) ${paneItem.label}`,
					description: paneItem.detail,
					detail: 'Pane History',
					id: `paneHistory:${paneItem.sourcePanePosition}`,
				});
			}
		}

		// Section: Git diff (placeholder entry)
		picks.push({ type: 'separator', label: 'OTHER' });
		picks.push({
			label: '$(diff) Git Diff (working tree)',
			description: 'Current uncommitted changes',
			detail: 'Git Diff',
			id: 'gitDiff:working',
		});

		const selected = await this.quickInputService.pick(picks, {
			placeHolder: 'Select context to attach (@)',
			canPickMany: true,
		});

		if (!selected) {
			return [];
		}

		const selectedArray = Array.isArray(selected) ? selected : [selected];
		const items: ForgeContextItem[] = [];

		for (const pick of selectedArray) {
			const item = await this.resolvePickedItem(pick);
			if (item) {
				items.push(item);
				this.addContextChip(panePosition, item, false);
			}
		}

		return items;
	}

	private async resolvePickedItem(pick: IQuickPickItem): Promise<ForgeContextItem | undefined> {
		const id = pick.id;
		if (!id) {
			return undefined;
		}

		const [type, ...rest] = id.split(':');
		const value = rest.join(':');

		switch (type) {
			case 'file': {
				try {
					const fileUri = URI.parse(value);
					return await this.fileContextProvider.resolveFile(fileUri);
				} catch (error) {
					this.logService.warn('[ForgeContextService] Failed to read file for context', error);
					return undefined;
				}
			}
			case 'selection': {
				const control = this.editorService.activeTextEditorControl;
				if (!isCodeEditor(control)) {
					return undefined;
				}
				const selection = control.getSelection();
				const model = control.getModel();
				if (!selection || !model || selection.isEmpty()) {
					return undefined;
				}
				const text = model.getValueInRange(selection);
				const resource = this.editorService.activeEditor?.resource;
				const fileName = resource?.path.split('/').pop() ?? 'unknown';
				return {
					type: ForgeContextType.Selection,
					label: fileName,
					detail: `${selection.startLineNumber}-${selection.endLineNumber}`,
					content: text,
					tokenEstimate: estimateTokens(text),
					uri: resource,
				};
			}
			case 'paneHistory': {
				const position = value as PanePosition;
				return this.paneHistoryProvider.resolvePaneHistory(position);
			}
			case 'gitDiff': {
				const folders = this.workspaceContextService.getWorkspace().folders;
				const workspaceRoot = folders.length > 0 ? folders[0].uri : URI.file('.');
				return this.gitDiffService.resolveGitDiff(workspaceRoot);
			}
			default:
				return undefined;
		}
	}

	private pruneStaleEntries(): void {
		const validPositions = new Set<string>(['default', 'tl', 'tr', 'bl', 'br']);
		const layout = this.layoutService.activeLayout;

		// code+ai keeps 'tr' (right pane is AI); quad keeps all four
		if (layout === 'code+ai') {
			for (const pos of ['tl', 'bl', 'br'] as const) {
				this._chipStore.delete(pos);
			}
		} else if (layout !== 'quad') {
			for (const pos of ['tl', 'tr', 'bl', 'br'] as const) {
				this._chipStore.delete(pos);
			}
		}

		// Remove entries for unknown keys
		for (const key of this._chipStore.keys()) {
			if (!validPositions.has(key)) {
				this._chipStore.delete(key);
			}
		}
	}
}

registerSingleton(IForgeContextService, ForgeContextService, InstantiationType.Delayed);
