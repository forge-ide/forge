/*---------------------------------------------------------------------------------------------
 * Forge - ForgeMcpView helper tests
 *--------------------------------------------------------------------------------------------*/

import * as assert from 'assert';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import { createServerRow, createToolRow, getLocalityLabel } from '../../browser/forgeMcpViewHelpers.js';
import { ForgeMcpServerStatusEntry } from '../../../../services/forge/common/forgeMcpService.js';
import { ForgeMcpServerStatus } from '../../../../services/forge/common/forgeMcpTypes.js';

function makeServer(overrides: Partial<ForgeMcpServerStatusEntry> = {}): ForgeMcpServerStatusEntry {
	return { name: 'filesystem', status: ForgeMcpServerStatus.Connected, toolCount: 5, disabled: false, transport: 'local', ...overrides };
}

suite('forgeMcpViewHelpers', () => {
	ensureNoDisposablesAreLeakedInTestSuite();

	suite('getLocalityLabel', () => {
		test('local transport -> "local"', () => assert.strictEqual(getLocalityLabel('local'), 'local'));
		test('remote transport -> "remote"', () => assert.strictEqual(getLocalityLabel('remote'), 'remote'));
	});

	suite('createServerRow', () => {
		test('renders server name', () => {
			const row = createServerRow(makeServer({ name: 'github' }));
			assert.ok(row.textContent?.includes('github'));
		});

		test('renders locality badge', () => {
			const row = createServerRow(makeServer({ transport: 'remote' }));
			const badge = row.querySelector('.forge-mcp-locality');
			assert.ok(badge);
			assert.ok(badge!.textContent?.includes('remote'));
		});

		test('adds disabled class when disabled', () => {
			const row = createServerRow(makeServer({ disabled: true }));
			assert.ok(row.classList.contains('forge-mcp-row--disabled'));
		});

		test('adds error class when status is Error', () => {
			const row = createServerRow(makeServer({ status: ForgeMcpServerStatus.Error }));
			assert.ok(row.classList.contains('forge-mcp-row--error'));
		});
	});

	suite('createToolRow', () => {
		test('renders tool name', () => {
			const row = createToolRow('read_file', 'Read a file from the filesystem');
			assert.ok(row.textContent?.includes('read_file'));
		});

		test('renders tool description', () => {
			const row = createToolRow('read_file', 'Read a file from the filesystem');
			assert.ok(row.textContent?.includes('Read a file from the filesystem'));
		});
	});
});
