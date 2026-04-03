import assert from 'assert';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import {
	ForgeMcpServerStatus,
	type ForgeMcpToolRecord,
	type ForgeMcpToolResultRecord,
	createToolRecord,
	createToolResultRecord
} from '../../common/forgeMcpTypes.js';

suite('ForgeMcpTypes', () => {
	ensureNoDisposablesAreLeakedInTestSuite();

	test('createToolRecord creates a well-formed record', () => {
		const record = createToolRecord('call_1', 'read_file', { path: '/tmp/test.txt' }, 'filesystem');
		assert.strictEqual(record.callId, 'call_1');
		assert.strictEqual(record.toolName, 'read_file');
		assert.strictEqual(record.serverName, 'filesystem');
		assert.strictEqual(record.status, 'pending');
		assert.ok(record.startedAt > 0);
	});

	test('createToolResultRecord creates result', () => {
		const result = createToolResultRecord('call_1', 'file contents here', false);
		assert.strictEqual(result.callId, 'call_1');
		assert.strictEqual(result.content, 'file contents here');
		assert.strictEqual(result.isError, false);
		assert.ok(result.completedAt > 0);
	});

	test('ForgeMcpServerStatus enum values', () => {
		assert.strictEqual(ForgeMcpServerStatus.Disconnected, 'disconnected');
		assert.strictEqual(ForgeMcpServerStatus.Connected, 'connected');
		assert.strictEqual(ForgeMcpServerStatus.Connecting, 'connecting');
		assert.strictEqual(ForgeMcpServerStatus.Error, 'error');
	});

	// Type shape checks
	const _typecheck: [ForgeMcpToolRecord, ForgeMcpToolResultRecord] = undefined!;
	void _typecheck;
});
