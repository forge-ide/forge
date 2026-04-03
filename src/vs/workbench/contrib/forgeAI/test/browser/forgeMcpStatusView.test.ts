/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import assert from 'assert';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import { FORGE_MCP_STATUS_VIEW_ID } from '../../browser/forgeMcpStatusView.js';

suite('ForgeMcpStatusView', () => {
	ensureNoDisposablesAreLeakedInTestSuite();

	test('has correct view ID', () => {
		assert.strictEqual(FORGE_MCP_STATUS_VIEW_ID, 'workbench.forgeAI.mcpStatusView');
	});
});
