/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import * as assert from 'assert';
// eslint-disable-next-line local/code-import-patterns
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../../base/test/common/utils.js';
import { getStepNumber, getTotalSteps, getNextState, getPrevState } from '../forgeOnboardingView.js';

suite('ForgeOnboardingView - navigation helpers', () => {
	ensureNoDisposablesAreLeakedInTestSuite();

	suite('getTotalSteps', () => {
		test('returns 5 when VS Code config detected', () => {
			assert.strictEqual(getTotalSteps(true), 5);
		});

		test('returns 4 when no VS Code config', () => {
			assert.strictEqual(getTotalSteps(false), 4);
		});
	});

	suite('getStepNumber', () => {
		test('welcome is always step 1', () => {
			assert.strictEqual(getStepNumber('welcome', true), 1);
			assert.strictEqual(getStepNumber('welcome', false), 1);
		});

		test('canvas is always step 2', () => {
			assert.strictEqual(getStepNumber('canvas', true), 2);
			assert.strictEqual(getStepNumber('canvas', false), 2);
		});

		test('import is step 3 when VS Code detected', () => {
			assert.strictEqual(getStepNumber('import', true), 3);
		});

		test('provider is step 4 with VS Code, step 3 without', () => {
			assert.strictEqual(getStepNumber('provider', true), 4);
			assert.strictEqual(getStepNumber('provider', false), 3);
		});

		test('mcp is step 5 with VS Code, step 4 without', () => {
			assert.strictEqual(getStepNumber('mcp', true), 5);
			assert.strictEqual(getStepNumber('mcp', false), 4);
		});
	});

	suite('getNextState', () => {
		test('welcome to canvas', () => {
			assert.strictEqual(getNextState('welcome', true), 'canvas');
			assert.strictEqual(getNextState('welcome', false), 'canvas');
		});

		test('canvas to import when VS Code detected', () => {
			assert.strictEqual(getNextState('canvas', true), 'import');
		});

		test('canvas to provider when no VS Code', () => {
			assert.strictEqual(getNextState('canvas', false), 'provider');
		});

		test('import to provider', () => {
			assert.strictEqual(getNextState('import', true), 'provider');
		});

		test('provider to mcp', () => {
			assert.strictEqual(getNextState('provider', true), 'mcp');
		});

		test('mcp to complete', () => {
			assert.strictEqual(getNextState('mcp', true), 'complete');
		});
	});

	suite('getPrevState', () => {
		test('canvas to welcome', () => {
			assert.strictEqual(getPrevState('canvas', true), 'welcome');
		});

		test('import to canvas', () => {
			assert.strictEqual(getPrevState('import', true), 'canvas');
		});

		test('provider to import when VS Code detected', () => {
			assert.strictEqual(getPrevState('provider', true), 'import');
		});

		test('provider to canvas when no VS Code', () => {
			assert.strictEqual(getPrevState('provider', false), 'canvas');
		});

		test('mcp to provider', () => {
			assert.strictEqual(getPrevState('mcp', true), 'provider');
		});

		test('welcome has no prev state', () => {
			assert.strictEqual(getPrevState('welcome', true), null);
		});
	});
});
