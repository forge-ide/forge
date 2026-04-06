import assert from 'assert';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import {
	ForgeAgentStatus,
	MAX_TURNS,
	createAgentTask,
	createAgentStep,
	createAgentTaskFromDefinition
} from '../../common/forgeAgentTypes.js';
import { AgentDefinition } from '../../common/forgeConfigResolutionTypes.js';

suite('ForgeAgentTypes', () => {
	ensureNoDisposablesAreLeakedInTestSuite();

	test('createAgentTask creates well-formed task', () => {
		const task = createAgentTask({
			name: 'File Refactorer',
			systemPrompt: 'You are a code refactoring agent.',
			taskDescription: 'Refactor src/providers/ to use a registry pattern.',
			providerName: 'anthropic',
			model: 'claude-sonnet-4-6'
		});
		assert.ok(task.id.startsWith('agent_'));
		assert.strictEqual(task.name, 'File Refactorer');
		assert.strictEqual(task.status, ForgeAgentStatus.Queued);
		assert.strictEqual(task.steps.length, 0);
		assert.strictEqual(task.currentTurn, 0);
		assert.strictEqual(task.maxTurns, 20);
	});

	test('createAgentTaskFromDefinition bridges .agents/ config to runtime task', () => {
		const definition: AgentDefinition = {
			name: 'code-reviewer',
			description: 'Reviews code for style and bugs',
			systemPrompt: 'You are a code reviewer. Focus on correctness.',
			tools: ['filesystem', 'github'],
			maxTurns: 10,
			provider: 'anthropic',
			model: 'claude-sonnet-4-6',
		};
		const task = createAgentTaskFromDefinition(definition, 'Review the auth module');
		assert.ok(task.id.startsWith('agent_'));
		assert.strictEqual(task.name, 'code-reviewer');
		assert.strictEqual(task.systemPrompt, 'You are a code reviewer. Focus on correctness.');
		assert.strictEqual(task.maxTurns, 10);
		assert.strictEqual(task.providerName, 'anthropic');
		assert.strictEqual(task.model, 'claude-sonnet-4-6');
		assert.strictEqual(task.taskDescription, 'Review the auth module');
		assert.deepStrictEqual(task.allowedTools, ['filesystem', 'github']);
	});

	test('createAgentTaskFromDefinition uses defaults when definition is minimal', () => {
		const definition: AgentDefinition = {
			name: 'minimal-agent',
			description: '',
			systemPrompt: 'Do stuff.',
		};
		const task = createAgentTaskFromDefinition(definition, 'Some task');
		assert.strictEqual(task.maxTurns, 20);
		assert.strictEqual(task.providerName, '');
		assert.strictEqual(task.model, '');
		assert.strictEqual(task.allowedTools, undefined);
	});

	test('createAgentStep creates a step record', () => {
		const step = createAgentStep('call_1', 'read_file', { path: '/a' });
		assert.strictEqual(step.toolCallId, 'call_1');
		assert.strictEqual(step.toolName, 'read_file');
		assert.strictEqual(step.status, 'pending');
		assert.ok(step.startedAt > 0);
	});

	test('createAgentTask clamps maxTurns to MAX_TURNS', () => {
		const task = createAgentTask({
			name: 'agent', systemPrompt: 'p', taskDescription: 't',
			providerName: 'p', model: 'm', maxTurns: 9999
		});
		assert.strictEqual(task.maxTurns, MAX_TURNS);
	});

	test('createAgentTask default maxTurns equals MAX_TURNS', () => {
		const task = createAgentTask({
			name: 'agent', systemPrompt: 'p', taskDescription: 't',
			providerName: 'p', model: 'm'
		});
		assert.strictEqual(task.maxTurns, MAX_TURNS);
	});

	test('ForgeAgentStatus enum values', () => {
		assert.strictEqual(ForgeAgentStatus.Queued, 'queued');
		assert.strictEqual(ForgeAgentStatus.Running, 'running');
		assert.strictEqual(ForgeAgentStatus.Completed, 'completed');
		assert.strictEqual(ForgeAgentStatus.Error, 'error');
		assert.strictEqual(ForgeAgentStatus.MaxTurnsReached, 'max_turns_reached');
	});
});
