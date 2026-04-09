/*---------------------------------------------------------------------------------------------
 * Forge - ForgeSessionContextService tests
 *--------------------------------------------------------------------------------------------*/

import * as assert from 'assert';
import { DisposableStore } from '../../../../../base/common/lifecycle.js';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import { ForgeSessionContextService } from '../../browser/forgeSessionContextService.js';

suite('ForgeSessionContextService', () => {
	let disposables: DisposableStore;

	setup(() => {
		disposables = new DisposableStore();
	});

	teardown(() => {
		disposables.dispose();
	});

	ensureNoDisposablesAreLeakedInTestSuite();

	test('getContext returns empty context for unknown session', () => {
		const service = disposables.add(new ForgeSessionContextService());
		const ctx = service.getContext('unknown');
		assert.deepStrictEqual(ctx.activeSkills, []);
		assert.deepStrictEqual(ctx.activeMcpServers, []);
	});

	test('setSkills updates active skills for session', () => {
		const service = disposables.add(new ForgeSessionContextService());
		service.setSkills('run-1', ['commit', 'tdd']);
		assert.deepStrictEqual(service.getContext('run-1').activeSkills, ['commit', 'tdd']);
	});

	test('setMcpServers updates active MCP servers for session', () => {
		const service = disposables.add(new ForgeSessionContextService());
		service.setMcpServers('run-1', ['filesystem', 'github']);
		assert.deepStrictEqual(service.getContext('run-1').activeMcpServers, ['filesystem', 'github']);
	});

	test('sessions are isolated', () => {
		const service = disposables.add(new ForgeSessionContextService());
		service.setSkills('run-1', ['commit']);
		service.setSkills('run-2', ['tdd']);
		assert.deepStrictEqual(service.getContext('run-1').activeSkills, ['commit']);
		assert.deepStrictEqual(service.getContext('run-2').activeSkills, ['tdd']);
	});

	test('onDidChangeContext fires with sessionId on setSkills', () => {
		const service = disposables.add(new ForgeSessionContextService());
		const fired: string[] = [];
		disposables.add(service.onDidChangeContext((id: string) => fired.push(id)));
		service.setSkills('run-1', ['commit']);
		assert.deepStrictEqual(fired, ['run-1']);
	});

	test('onDidChangeContext fires with sessionId on setMcpServers', () => {
		const service = disposables.add(new ForgeSessionContextService());
		const fired: string[] = [];
		disposables.add(service.onDidChangeContext((id: string) => fired.push(id)));
		service.setMcpServers('run-1', ['filesystem']);
		assert.deepStrictEqual(fired, ['run-1']);
	});

	test('clearContext removes session data', () => {
		const service = disposables.add(new ForgeSessionContextService());
		service.setSkills('run-1', ['commit']);
		service.clearContext('run-1');
		assert.deepStrictEqual(service.getContext('run-1').activeSkills, []);
	});
});
