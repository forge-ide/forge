import assert from 'assert';
// eslint-disable-next-line local/code-import-patterns
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../../base/test/common/utils.js';
import {
	ForgeChatTextPart,
	ForgeChatToolCallPart,
	ForgeChatToolResultPart,
	ForgeChatAgentProgressPart,
	isTextPart,
	isToolCallPart,
	isToolResultPart,
	isAgentProgressPart,
	ForgeAssistantMessage
} from '../forgeChatMessageTypes.js';

suite('ForgeChatMessageTypes', () => {
	ensureNoDisposablesAreLeakedInTestSuite();

	test('ForgeChatTextPart', () => {
		const part: ForgeChatTextPart = { type: 'text', content: 'Hello world' };
		assert.ok(isTextPart(part));
		assert.ok(!isToolCallPart(part));
	});

	test('ForgeChatToolCallPart', () => {
		const part: ForgeChatToolCallPart = {
			type: 'tool_call',
			callId: 'call_1',
			toolName: 'read_file',
			input: { path: '/tmp/test.txt' },
			serverName: 'filesystem',
			status: 'running'
		};
		assert.ok(isToolCallPart(part));
		assert.strictEqual(part.toolName, 'read_file');
	});

	test('ForgeChatToolResultPart', () => {
		const part: ForgeChatToolResultPart = {
			type: 'tool_result',
			callId: 'call_1',
			content: 'file contents',
			isError: false,
			durationMs: 120
		};
		assert.ok(isToolResultPart(part));
		assert.strictEqual(part.durationMs, 120);
	});

	test('ForgeChatAgentProgressPart', () => {
		const part: ForgeChatAgentProgressPart = {
			type: 'agent_progress',
			agentId: 'agent_1',
			agentName: 'File Refactorer',
			status: 'running',
			currentStep: 2,
			totalSteps: 5,
			stepLabel: 'Reading src/index.ts'
		};
		assert.ok(isAgentProgressPart(part));
		assert.strictEqual(part.currentStep, 2);
	});

	test('ForgeAssistantMessage holds mixed content parts', () => {
		const msg: ForgeAssistantMessage = {
			role: 'assistant',
			parts: [
				{ type: 'text', content: 'I will read the file.' },
				{ type: 'tool_call', callId: 'c1', toolName: 'read_file', input: { path: '/a' }, serverName: 'fs', status: 'completed' },
				{ type: 'tool_result', callId: 'c1', content: 'data', isError: false, durationMs: 50 },
				{ type: 'text', content: 'Here is what I found.' }
			]
		};
		assert.strictEqual(msg.parts.length, 4);
		assert.ok(isTextPart(msg.parts[0]));
		assert.ok(isToolCallPart(msg.parts[1]));
	});
});
