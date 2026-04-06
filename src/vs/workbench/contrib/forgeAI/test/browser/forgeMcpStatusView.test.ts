/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import assert from 'assert';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import { FORGE_MCP_STATUS_VIEW_ID } from '../../browser/forgeMcpStatusView.js';
import {
	getServerStatusClass,
	getToolCountText,
	createEmptyServersState,
	createServerRow,
} from '../../browser/forgeMcpStatusViewHelpers.js';
import { ForgeMcpServerStatus } from '../../../../services/forge/common/forgeMcpTypes.js';
import { ForgeMcpServerStatusEntry } from '../../../../services/forge/common/forgeMcpService.js';

function makeServer(overrides: Partial<ForgeMcpServerStatusEntry> = {}): ForgeMcpServerStatusEntry {
	return {
		name: 'test-server',
		status: ForgeMcpServerStatus.Connected,
		toolCount: 3,
		disabled: false,
		...overrides,
	};
}

suite('ForgeMcpStatusView', () => {
	ensureNoDisposablesAreLeakedInTestSuite();

	test('has correct view ID', () => {
		assert.strictEqual(FORGE_MCP_STATUS_VIEW_ID, 'workbench.forgeAI.mcpStatusView');
	});

	// --- getServerStatusClass ---

	test('getServerStatusClass Connected, not disabled → "connected"', () => {
		assert.strictEqual(getServerStatusClass(ForgeMcpServerStatus.Connected, false), 'connected');
	});

	test('getServerStatusClass Connecting, not disabled → "connecting"', () => {
		assert.strictEqual(getServerStatusClass(ForgeMcpServerStatus.Connecting, false), 'connecting');
	});

	test('getServerStatusClass Error, not disabled → "error"', () => {
		assert.strictEqual(getServerStatusClass(ForgeMcpServerStatus.Error, false), 'error');
	});

	test('getServerStatusClass Disconnected, not disabled → "disconnected"', () => {
		assert.strictEqual(getServerStatusClass(ForgeMcpServerStatus.Disconnected, false), 'disconnected');
	});

	test('getServerStatusClass disabled=true → "disabled" regardless of status', () => {
		assert.strictEqual(getServerStatusClass(ForgeMcpServerStatus.Connected, true), 'disabled');
		assert.strictEqual(getServerStatusClass(ForgeMcpServerStatus.Error, true), 'disabled');
	});

	// --- getToolCountText ---

	test('getToolCountText disabled=false → "<n> tools"', () => {
		assert.strictEqual(getToolCountText(5, false), '5 tools');
		assert.strictEqual(getToolCountText(0, false), '0 tools');
	});

	test('getToolCountText disabled=true → "disabled"', () => {
		assert.strictEqual(getToolCountText(5, true), 'disabled');
	});

	// --- createEmptyServersState ---

	test('createEmptyServersState returns element with correct class and text', () => {
		const el = createEmptyServersState();
		assert.strictEqual(el.className, 'forge-mcp-empty');
		assert.strictEqual(el.textContent, 'No MCP servers configured');
	});

	// --- createServerRow ---

	test('createServerRow Connected → "connected" dot class, tool count text', () => {
		const row = createServerRow(makeServer({ toolCount: 4 }), () => { });
		assert.ok(row.querySelector('.forge-mcp-server-dot.connected'), 'expected .connected dot');
		assert.strictEqual(row.querySelector('.forge-mcp-tool-count')?.textContent, '4 tools');
	});

	test('createServerRow Connecting → "connecting" dot class', () => {
		const row = createServerRow(makeServer({ status: ForgeMcpServerStatus.Connecting }), () => { });
		assert.ok(row.querySelector('.forge-mcp-server-dot.connecting'));
	});

	test('createServerRow Error → "error" dot class', () => {
		const row = createServerRow(makeServer({ status: ForgeMcpServerStatus.Error }), () => { });
		assert.ok(row.querySelector('.forge-mcp-server-dot.error'));
	});

	test('createServerRow Disconnected → "disconnected" dot class', () => {
		const row = createServerRow(makeServer({ status: ForgeMcpServerStatus.Disconnected }), () => { });
		assert.ok(row.querySelector('.forge-mcp-server-dot.disconnected'));
	});

	test('createServerRow disabled=true → "disabled" dot class, "disabled" text, "Enable" button', () => {
		const row = createServerRow(makeServer({ disabled: true }), () => { });
		assert.ok(row.querySelector('.forge-mcp-server-dot.disabled'), 'expected .disabled dot');
		assert.strictEqual(row.querySelector('.forge-mcp-tool-count')?.textContent, 'disabled');
		assert.strictEqual(row.querySelector<HTMLButtonElement>('.forge-mcp-server-toggle')!.textContent, 'Enable');
	});

	test('createServerRow disabled=false → "Disable" button', () => {
		const row = createServerRow(makeServer(), () => { });
		assert.strictEqual(row.querySelector<HTMLButtonElement>('.forge-mcp-server-toggle')!.textContent, 'Disable');
	});

	test('createServerRow toggle click fires onToggle callback', () => {
		let called = false;
		const row = createServerRow(makeServer(), () => { called = true; });
		row.querySelector<HTMLButtonElement>('.forge-mcp-server-toggle')!.click();
		assert.strictEqual(called, true);
	});
});
