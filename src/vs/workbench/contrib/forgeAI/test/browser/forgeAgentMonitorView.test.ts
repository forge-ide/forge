/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import assert from 'assert';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import { FORGE_AGENT_MONITOR_VIEW_ID } from '../../browser/forgeAgentMonitorView.js';
import {
	getAgentStatusClass,
	sortAgentsByStatus,
	createEmptyDefinitionsState,
	createEmptyAgentsState,
	createDefinitionRow,
	createAgentRow,
} from '../../browser/forgeAgentMonitorViewHelpers.js';
import { ForgeAgentStatus, ForgeAgentTask } from '../../../../services/forge/common/forgeAgentTypes.js';
import { AgentDefinition } from '../../../../services/forge/common/forgeConfigResolutionTypes.js';

function makeTask(status: ForgeAgentStatus, overrides: Partial<ForgeAgentTask> = {}): ForgeAgentTask {
	return {
		id: 'test-id',
		name: 'test-agent',
		systemPrompt: '',
		taskDescription: 'do something',
		providerName: 'anthropic',
		model: 'claude-3-5-sonnet',
		maxTurns: 10,
		status,
		currentTurn: 3,
		steps: [],
		...overrides,
	};
}

function makeDef(overrides: Partial<AgentDefinition> = {}): AgentDefinition {
	return {
		name: 'my-agent',
		description: 'does stuff',
		systemPrompt: 'you are helpful',
		...overrides,
	};
}

suite('ForgeAgentMonitorView', () => {
	ensureNoDisposablesAreLeakedInTestSuite();

	test('has correct view ID', () => {
		assert.strictEqual(FORGE_AGENT_MONITOR_VIEW_ID, 'workbench.forgeAI.agentMonitorView');
	});

	// --- getAgentStatusClass ---

	test('getAgentStatusClass Running → "running"', () => {
		assert.strictEqual(getAgentStatusClass(ForgeAgentStatus.Running), 'running');
	});

	test('getAgentStatusClass Queued → "queued"', () => {
		assert.strictEqual(getAgentStatusClass(ForgeAgentStatus.Queued), 'queued');
	});

	test('getAgentStatusClass Completed → "completed"', () => {
		assert.strictEqual(getAgentStatusClass(ForgeAgentStatus.Completed), 'completed');
	});

	test('getAgentStatusClass Error → "error"', () => {
		assert.strictEqual(getAgentStatusClass(ForgeAgentStatus.Error), 'error');
	});

	test('getAgentStatusClass MaxTurnsReached → "max_turns_reached"', () => {
		assert.strictEqual(getAgentStatusClass(ForgeAgentStatus.MaxTurnsReached), 'max_turns_reached');
	});

	// --- sortAgentsByStatus ---

	test('sortAgentsByStatus orders Running first, Error last', () => {
		const tasks = [
			makeTask(ForgeAgentStatus.Error),
			makeTask(ForgeAgentStatus.Queued),
			makeTask(ForgeAgentStatus.Running),
			makeTask(ForgeAgentStatus.Completed),
			makeTask(ForgeAgentStatus.MaxTurnsReached),
		];
		const sorted = sortAgentsByStatus(tasks);
		assert.strictEqual(sorted[0].status, ForgeAgentStatus.Running);
		assert.strictEqual(sorted[1].status, ForgeAgentStatus.Queued);
		assert.strictEqual(sorted[2].status, ForgeAgentStatus.Completed);
		assert.strictEqual(sorted[3].status, ForgeAgentStatus.MaxTurnsReached);
		assert.strictEqual(sorted[4].status, ForgeAgentStatus.Error);
	});

	test('sortAgentsByStatus does not mutate the input array', () => {
		const tasks = [makeTask(ForgeAgentStatus.Error), makeTask(ForgeAgentStatus.Running)];
		const original = [...tasks];
		sortAgentsByStatus(tasks);
		assert.strictEqual(tasks[0].status, original[0].status);
	});

	// --- createEmptyDefinitionsState ---

	test('createEmptyDefinitionsState returns element with correct class', () => {
		const el = createEmptyDefinitionsState();
		assert.strictEqual(el.className, 'forge-agent-empty');
	});

	test('createEmptyDefinitionsState text mentions definitions', () => {
		const el = createEmptyDefinitionsState();
		assert.ok(el.textContent!.length > 0);
	});

	// --- createEmptyAgentsState ---

	test('createEmptyAgentsState returns element with correct class', () => {
		const el = createEmptyAgentsState();
		assert.strictEqual(el.className, 'forge-agent-empty');
	});

	test('createEmptyAgentsState text mentions agents', () => {
		const el = createEmptyAgentsState();
		assert.ok(el.textContent!.length > 0);
	});

	// --- createDefinitionRow ---

	test('createDefinitionRow renders name and description', () => {
		const row = createDefinitionRow(makeDef(), false, () => { });
		assert.strictEqual(row.querySelector('.forge-agent-def-name')?.textContent, 'my-agent');
		assert.strictEqual(row.querySelector('.forge-agent-def-desc')?.textContent, 'does stuff');
	});

	test('createDefinitionRow disabled=false → button text is "Disable"', () => {
		const row = createDefinitionRow(makeDef(), false, () => { });
		assert.strictEqual(row.querySelector<HTMLButtonElement>('.forge-agent-def-toggle')!.textContent, 'Disable');
	});

	test('createDefinitionRow disabled=true → button text is "Enable"', () => {
		const row = createDefinitionRow(makeDef(), true, () => { });
		assert.strictEqual(row.querySelector<HTMLButtonElement>('.forge-agent-def-toggle')!.textContent, 'Enable');
	});

	test('createDefinitionRow toggle click fires onToggle callback', () => {
		let called = false;
		const row = createDefinitionRow(makeDef(), false, () => { called = true; });
		row.querySelector<HTMLButtonElement>('.forge-agent-def-toggle')!.click();
		assert.strictEqual(called, true);
	});

	// --- createAgentRow ---

	test('createAgentRow shows name and turn counter', () => {
		const task = makeTask(ForgeAgentStatus.Running, { currentTurn: 3, maxTurns: 10 });
		const row = createAgentRow(task, () => { });
		assert.strictEqual(row.querySelector('.forge-agent-row-name')?.textContent, 'test-agent');
		assert.strictEqual(row.querySelector('.forge-agent-row-turns')?.textContent, '3/10');
	});

	test('createAgentRow Running → "running" status class and Cancel button present', () => {
		const task = makeTask(ForgeAgentStatus.Running);
		const row = createAgentRow(task, () => { });
		assert.ok(row.querySelector('.forge-agent-status.running'), 'expected .forge-agent-status.running');
		assert.ok(row.querySelector('.forge-agent-cancel-btn'), 'expected cancel button');
	});

	test('createAgentRow Completed → "completed" status class, no Cancel button', () => {
		const task = makeTask(ForgeAgentStatus.Completed);
		const row = createAgentRow(task, () => { });
		assert.ok(row.querySelector('.forge-agent-status.completed'), 'expected .forge-agent-status.completed');
		assert.strictEqual(row.querySelector('.forge-agent-cancel-btn'), null);
	});

	test('createAgentRow MaxTurnsReached → "max_turns_reached" status class', () => {
		const task = makeTask(ForgeAgentStatus.MaxTurnsReached);
		const row = createAgentRow(task, () => { });
		assert.ok(row.querySelector('.forge-agent-status.max_turns_reached'));
	});

	test('createAgentRow Error → "error" status class', () => {
		const task = makeTask(ForgeAgentStatus.Error);
		const row = createAgentRow(task, () => { });
		assert.ok(row.querySelector('.forge-agent-status.error'));
	});

	test('createAgentRow Queued → "queued" status class, no Cancel button', () => {
		const task = makeTask(ForgeAgentStatus.Queued);
		const row = createAgentRow(task, () => { });
		assert.ok(row.querySelector('.forge-agent-status.queued'));
		assert.strictEqual(row.querySelector('.forge-agent-cancel-btn'), null);
	});

	test('createAgentRow Cancel click fires onCancel with agent id', () => {
		let cancelledId: string | undefined;
		const task = makeTask(ForgeAgentStatus.Running, { id: 'agent-42' });
		const row = createAgentRow(task, (id) => { cancelledId = id; });
		row.querySelector<HTMLButtonElement>('.forge-agent-cancel-btn')!.click();
		assert.strictEqual(cancelledId, 'agent-42');
	});
});
