import assert from 'assert';
import { IConfigurationService } from '../../../../../platform/configuration/common/configuration.js';
import { NullLogService } from '../../../../../platform/log/common/log.js';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import { ForgeConfigService } from '../../common/forgeConfigService.js';

function makeConfigService(initial: Record<string, unknown> = {}): {
	mockConfig: IConfigurationService;
	updates: Array<{ key: string; value: unknown }>;
} {
	const store: Record<string, unknown> = { ...initial };
	const updates: Array<{ key: string; value: unknown }> = [];
	const listeners: Array<(e: { affectsConfiguration(k: string): boolean }) => void> = [];

	const mockConfig = {
		getValue: <T>(key: string) => (store[key] as T),
		updateValue: async (key: string, value: unknown) => {
			store[key] = value;
			updates.push({ key, value });
			for (const l of listeners) {
				l({ affectsConfiguration: (k: string) => k === key });
			}
		},
		onDidChangeConfiguration: (l: (e: { affectsConfiguration(k: string): boolean }) => void) => {
			listeners.push(l);
			return { dispose: () => { listeners.splice(listeners.indexOf(l), 1); } };
		},
	};

	return { mockConfig: mockConfig as unknown as IConfigurationService, updates };
}

suite('ForgeConfigService', () => {
	const ds = ensureNoDisposablesAreLeakedInTestSuite();

	test('getConfig() reads values from IConfigurationService', () => {
		const { mockConfig } = makeConfigService({
			'forge.defaultProvider': 'anthropic',
			'forge.defaultModel': 'claude-opus-4-5',
			'forge.stream': false,
			'forge.providers': [{ name: 'anthropic', models: [] }],
		});
		const sut = ds.add(new ForgeConfigService(mockConfig, new NullLogService()));

		const config = sut.getConfig();

		assert.strictEqual(config.defaultProvider, 'anthropic');
		assert.strictEqual(config.defaultModel, 'claude-opus-4-5');
		assert.strictEqual(config.stream, false);
		assert.deepStrictEqual(config.providers, [{ name: 'anthropic', models: [] }]);
	});

	test('getConfig() returns safe defaults when values are absent', () => {
		const { mockConfig } = makeConfigService({});
		const sut = ds.add(new ForgeConfigService(mockConfig, new NullLogService()));

		const config = sut.getConfig();

		assert.strictEqual(config.defaultProvider, '');
		assert.strictEqual(config.stream, true);
		assert.deepStrictEqual(config.providers, []);
	});

	test('updateConfig() writes each supplied field to IConfigurationService', async () => {
		const { mockConfig, updates } = makeConfigService({});
		const sut = ds.add(new ForgeConfigService(mockConfig, new NullLogService()));

		await sut.updateConfig({ defaultProvider: 'openai', stream: false });

		assert.strictEqual(updates.length, 2);
		assert.ok(updates.some(u => u.key === 'forge.defaultProvider' && u.value === 'openai'));
		assert.ok(updates.some(u => u.key === 'forge.stream' && u.value === false));
	});

	test('updateConfig() does not write unspecified fields', async () => {
		const { mockConfig, updates } = makeConfigService({});
		const sut = ds.add(new ForgeConfigService(mockConfig, new NullLogService()));

		await sut.updateConfig({ defaultProvider: 'openai' });

		assert.strictEqual(updates.length, 1);
		assert.strictEqual(updates[0].key, 'forge.defaultProvider');
	});

	test('onDidChange fires when a forge key changes', async () => {
		const { mockConfig } = makeConfigService({});
		const sut = ds.add(new ForgeConfigService(mockConfig, new NullLogService()));

		let firedCount = 0;
		ds.add(sut.onDidChange(() => { firedCount++; }));

		await sut.updateConfig({ defaultProvider: 'anthropic' });

		assert.strictEqual(firedCount, 1);
	});

	test('onDidChange does not fire when an unrelated key changes', () => {
		const store: Record<string, unknown> = {};
		const listeners: Array<(e: { affectsConfiguration(k: string): boolean }) => void> = [];
		const mockConfig = {
			getValue: <T>(key: string) => store[key] as T,
			updateValue: async () => { },
			onDidChangeConfiguration: (l: (e: { affectsConfiguration(k: string): boolean }) => void) => {
				listeners.push(l);
				return { dispose: () => { } };
			},
		};
		const sut = ds.add(new ForgeConfigService(mockConfig as unknown as IConfigurationService, new NullLogService()));

		let firedCount = 0;
		ds.add(sut.onDidChange(() => { firedCount++; }));

		for (const l of listeners) {
			l({ affectsConfiguration: (k: string) => k === 'editor.fontSize' });
		}

		assert.strictEqual(firedCount, 0);
	});
});
