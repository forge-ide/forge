/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import assert from 'assert';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import { FORGE_AGENT_MONITOR_VIEW_ID } from '../../browser/forgeAgentMonitorView.js';

suite('ForgeAgentMonitorView', () => {
	ensureNoDisposablesAreLeakedInTestSuite();

	test('has correct view ID', () => {
		assert.strictEqual(FORGE_AGENT_MONITOR_VIEW_ID, 'workbench.forgeAI.agentMonitorView');
	});
});
