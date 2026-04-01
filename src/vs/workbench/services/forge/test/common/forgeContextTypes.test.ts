/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import assert from 'assert';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import { URI } from '../../../../../base/common/uri.js';
import { ForgeContextType, type ForgeContextItem, type ForgeContextChip, type IForgeContextBudget } from '../../common/forgeContextTypes.js';

suite('ForgeContextTypes', () => {

	ensureNoDisposablesAreLeakedInTestSuite();

	test('ForgeContextType enum values are distinct strings', () => {
		const values = [
			ForgeContextType.File,
			ForgeContextType.Selection,
			ForgeContextType.GitDiff,
			ForgeContextType.Symbol,
			ForgeContextType.PaneHistory,
			ForgeContextType.ActiveEditor,
		];

		// All values should be strings
		for (const v of values) {
			assert.strictEqual(typeof v, 'string', `Expected string, got ${typeof v}: ${v}`);
		}

		// All values should be distinct
		const unique = new Set(values);
		assert.strictEqual(unique.size, values.length, 'All ForgeContextType values must be distinct');

		// Verify specific values match the type union
		assert.strictEqual(ForgeContextType.File, 'file');
		assert.strictEqual(ForgeContextType.Selection, 'selection');
		assert.strictEqual(ForgeContextType.GitDiff, 'gitDiff');
		assert.strictEqual(ForgeContextType.Symbol, 'symbol');
		assert.strictEqual(ForgeContextType.PaneHistory, 'paneHistory');
		assert.strictEqual(ForgeContextType.ActiveEditor, 'activeEditor');
	});

	test('ForgeContextItem interface accepts all required fields', () => {
		const item: ForgeContextItem = {
			type: ForgeContextType.File,
			label: 'test-file.ts',
			content: 'const x = 1;',
			tokenEstimate: 4,
		};

		assert.strictEqual(item.type, 'file');
		assert.strictEqual(item.label, 'test-file.ts');
		assert.strictEqual(item.content, 'const x = 1;');
		assert.strictEqual(item.tokenEstimate, 4);
		assert.strictEqual(item.detail, undefined);
		assert.strictEqual(item.uri, undefined);
		assert.strictEqual(item.sourcePanePosition, undefined);

		// With all optional fields
		const fullItem: ForgeContextItem = {
			type: ForgeContextType.Selection,
			label: 'selection in main.ts',
			detail: 'lines 10-20',
			content: 'function foo() {}',
			tokenEstimate: 5,
			uri: URI.file('/workspace/main.ts'),
			sourcePanePosition: 'tl',
		};

		assert.strictEqual(fullItem.type, 'selection');
		assert.strictEqual(fullItem.detail, 'lines 10-20');
		assert.ok(fullItem.uri);
		assert.strictEqual(fullItem.sourcePanePosition, 'tl');
	});

	test('ForgeContextChip wraps item with automatic flag', () => {
		const item: ForgeContextItem = {
			type: ForgeContextType.ActiveEditor,
			label: 'index.ts',
			content: 'export default {};',
			tokenEstimate: 4,
		};

		const manualChip: ForgeContextChip = { item, automatic: false };
		const autoChip: ForgeContextChip = { item, automatic: true };

		assert.strictEqual(manualChip.automatic, false);
		assert.strictEqual(autoChip.automatic, true);
		assert.strictEqual(manualChip.item.type, 'activeEditor');
	});

	test('IForgeContextBudget tracks token usage and dropped items', () => {
		const chip: ForgeContextChip = {
			item: {
				type: ForgeContextType.File,
				label: 'big-file.ts',
				content: 'x'.repeat(4000),
				tokenEstimate: 1000,
			},
			automatic: false,
		};

		const budget: IForgeContextBudget = {
			maxTokens: 500,
			usedTokens: 0,
			items: [],
			droppedCount: 1,
		};

		assert.strictEqual(budget.maxTokens, 500);
		assert.strictEqual(budget.usedTokens, 0);
		assert.strictEqual(budget.items.length, 0);
		assert.strictEqual(budget.droppedCount, 1);

		const fullBudget: IForgeContextBudget = {
			maxTokens: 2000,
			usedTokens: 1000,
			items: [chip],
			droppedCount: 0,
		};

		assert.strictEqual(fullBudget.usedTokens, 1000);
		assert.strictEqual(fullBudget.items.length, 1);
		assert.strictEqual(fullBudget.droppedCount, 0);
	});
});
