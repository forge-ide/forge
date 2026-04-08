import assert from 'assert';
import { DisposableStore } from '../../../../../base/common/lifecycle.js';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import { InMemoryStorageService } from '../../../../../platform/storage/common/storage.js';
import { NullLogService } from '../../../../../platform/log/common/log.js';
import type { IFileService } from '../../../../../platform/files/common/files.js';
import type { INativeHostService } from '../../../../../platform/native/common/native.js';
import { ForgeOnboardingServiceImpl } from '../../electron-browser/forgeOnboardingService.js';

suite('ForgeOnboardingService - MCP selections', () => {

	let disposables: DisposableStore;

	setup(() => {
		disposables = new DisposableStore();
	});

	teardown(() => {
		disposables.dispose();
	});

	ensureNoDisposablesAreLeakedInTestSuite();

	function createService() {
		const service = new ForgeOnboardingServiceImpl(
			disposables.add(new InMemoryStorageService()),
			null as unknown as IFileService,
			new NullLogService(),
			null as unknown as INativeHostService,
		);
		disposables.add(service);
		return service;
	}

	test('getMCPSelections returns empty array when nothing saved', async () => {
		const service = createService();
		const result = await service.getMCPSelections();
		assert.deepStrictEqual(result, []);
	});

	test('saveMCPSelections and getMCPSelections round-trip', async () => {
		const service = createService();
		await service.saveMCPSelections(['filesystem', 'github']);
		const result = await service.getMCPSelections();
		assert.deepStrictEqual(result, ['filesystem', 'github']);
	});

	test('saveMCPSelections overwrites previous selections', async () => {
		const service = createService();
		await service.saveMCPSelections(['filesystem', 'github']);
		await service.saveMCPSelections(['browser']);
		const result = await service.getMCPSelections();
		assert.deepStrictEqual(result, ['browser']);
	});

	test('saveMCPSelections accepts empty array', async () => {
		const service = createService();
		await service.saveMCPSelections(['filesystem']);
		await service.saveMCPSelections([]);
		const result = await service.getMCPSelections();
		assert.deepStrictEqual(result, []);
	});
});
