import assert from 'assert';
// eslint-disable-next-line local/code-import-patterns
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../../base/test/common/utils.js';
import {
	renderToolCallCard,
	updateToolCallCard,
	renderToolResultInCard
} from '../forgeChatToolCallRenderer.js';
import { ForgeChatToolCallPart, ForgeChatToolResultPart } from '../forgeChatMessageTypes.js';

suite('ForgeChatToolCallRenderer', () => {
	ensureNoDisposablesAreLeakedInTestSuite();

	test('renderToolCallCard creates DOM with tool name', () => {
		const part: ForgeChatToolCallPart = {
			type: 'tool_call',
			callId: 'call_1',
			toolName: 'read_file',
			input: { path: '/tmp/test.txt' },
			serverName: 'filesystem',
			status: 'running'
		};
		const el = renderToolCallCard(part);
		assert.ok(el.classList.contains('forge-tool-call-card'));
		assert.ok(el.querySelector('.forge-tool-call-name')?.textContent?.includes('read_file'));
		assert.ok(el.querySelector('.forge-tool-call-server')?.textContent?.includes('filesystem'));
	});

	test('renderToolCallCard shows status indicator', () => {
		const part: ForgeChatToolCallPart = {
			type: 'tool_call',
			callId: 'call_1',
			toolName: 'read_file',
			input: { path: '/tmp/test.txt' },
			serverName: 'filesystem',
			status: 'running'
		};
		const el = renderToolCallCard(part);
		const status = el.querySelector('.forge-tool-call-status');
		assert.ok(status);
		assert.ok(status!.classList.contains('running'));
	});

	test('renderToolCallCard has collapsible arguments', () => {
		const part: ForgeChatToolCallPart = {
			type: 'tool_call',
			callId: 'call_1',
			toolName: 'read_file',
			input: { path: '/tmp/test.txt' },
			serverName: 'filesystem',
			status: 'pending'
		};
		const el = renderToolCallCard(part);
		const argsSection = el.querySelector('.forge-tool-call-args');
		assert.ok(argsSection);
		assert.ok(argsSection!.classList.contains('collapsed'));
	});

	test('updateToolCallCard changes status', () => {
		const part: ForgeChatToolCallPart = {
			type: 'tool_call',
			callId: 'call_1',
			toolName: 'read_file',
			input: {},
			serverName: 'filesystem',
			status: 'running'
		};
		const el = renderToolCallCard(part);
		updateToolCallCard(el, 'completed');
		const status = el.querySelector('.forge-tool-call-status');
		assert.ok(status!.classList.contains('completed'));
		assert.ok(!status!.classList.contains('running'));
	});

	test('renderToolResultInCard appends result preview', () => {
		const part: ForgeChatToolCallPart = {
			type: 'tool_call',
			callId: 'call_1',
			toolName: 'read_file',
			input: {},
			serverName: 'filesystem',
			status: 'completed'
		};
		const el = renderToolCallCard(part);
		const result: ForgeChatToolResultPart = {
			type: 'tool_result',
			callId: 'call_1',
			content: 'file contents here',
			isError: false,
			durationMs: 85
		};
		renderToolResultInCard(el, result);
		assert.ok(el.querySelector('.forge-tool-call-result'));
		assert.ok(el.querySelector('.forge-tool-call-duration')?.textContent?.includes('85ms'));
	});

	test('renderToolResultInCard shows error state', () => {
		const part: ForgeChatToolCallPart = {
			type: 'tool_call',
			callId: 'call_1',
			toolName: 'read_file',
			input: {},
			serverName: 'filesystem',
			status: 'error'
		};
		const el = renderToolCallCard(part);
		const result: ForgeChatToolResultPart = {
			type: 'tool_result',
			callId: 'call_1',
			content: 'Permission denied',
			isError: true
		};
		renderToolResultInCard(el, result);
		const resultEl = el.querySelector('.forge-tool-call-result');
		assert.ok(resultEl!.classList.contains('error'));
	});
});
