/*---------------------------------------------------------------------------------------------
 * Forge - ForgeAgentsView helper tests
 *--------------------------------------------------------------------------------------------*/

import * as assert from 'assert';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import {
	createDefinitionRow,
	createRunningAgentRow,
	createChipElement,
	getAgentStatusLabel,
} from '../../browser/forgeAgentsViewHelpers.js';
import { AgentDefinition } from '../../../../services/forge/common/forgeConfigResolutionTypes.js';
import { ForgeAgentStatus } from '../../../../services/forge/common/forgeAgentTypes.js';

function makeDef(overrides: Partial<AgentDefinition> = {}): AgentDefinition {
	return { name: 'test-agent', description: 'A test agent', systemPrompt: 'You are a test agent.', ...overrides };
}

suite('forgeAgentsViewHelpers', () => {
	ensureNoDisposablesAreLeakedInTestSuite();

	suite('createDefinitionRow', () => {
		test('renders agent name', () => {
			const row = createDefinitionRow(makeDef({ name: 'my-coder' }), false);
			assert.ok(row.textContent?.includes('my-coder'));
		});

		test('adds live indicator class when agent is running', () => {
			const row = createDefinitionRow(makeDef(), true);
			assert.ok(row.querySelector('.forge-agent-live-dot'));
		});

		test('no live indicator when agent is not running', () => {
			const row = createDefinitionRow(makeDef(), false);
			assert.strictEqual(row.querySelector('.forge-agent-live-dot'), null);
		});
	});

	suite('createRunningAgentRow', () => {
		test('renders agent id', () => {
			const row = createRunningAgentRow('run-1', 'my-coder', ForgeAgentStatus.Running, 4, 20);
			assert.ok(row.textContent?.includes('my-coder'));
		});

		test('renders turn counter', () => {
			const row = createRunningAgentRow('run-1', 'my-coder', ForgeAgentStatus.Running, 4, 20);
			assert.ok(row.textContent?.includes('4'));
			assert.ok(row.textContent?.includes('20'));
		});
	});

	suite('createChipElement', () => {
		test('renders chip with label', () => {
			const chip = createChipElement('commit', () => { });
			assert.ok(chip.textContent?.includes('commit'));
		});

		test('calls onRemove when remove button clicked', () => {
			let called = false;
			const chip = createChipElement('commit', () => { called = true; });
			const btn = chip.querySelector('.forge-chip-remove') as HTMLElement;
			btn.click();
			assert.strictEqual(called, true);
		});
	});

	suite('getAgentStatusLabel', () => {
		test('Running -> "Running"', () => assert.strictEqual(getAgentStatusLabel(ForgeAgentStatus.Running), 'Running'));
		test('Completed -> "Completed"', () => assert.strictEqual(getAgentStatusLabel(ForgeAgentStatus.Completed), 'Completed'));
		test('Error -> "Error"', () => assert.strictEqual(getAgentStatusLabel(ForgeAgentStatus.Error), 'Error'));
	});
});
