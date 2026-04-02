/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import assert from 'assert';
import { VSBuffer } from '../../../../../base/common/buffer.js';
import { Event } from '../../../../../base/common/event.js';
import { DisposableStore } from '../../../../../base/common/lifecycle.js';
import { Schemas } from '../../../../../base/common/network.js';
import { URI } from '../../../../../base/common/uri.js';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import { FileService } from '../../../../../platform/files/common/fileService.js';
import { InMemoryFileSystemProvider } from '../../../../../platform/files/common/inMemoryFilesystemProvider.js';
import { NullLogService } from '../../../../../platform/log/common/log.js';
import { ForgeConfigService, type ForgeConfig } from '../../common/forgeConfigService.js';
import type { IEnvironmentService } from '../../../../../platform/environment/common/environment.js';
import type { IFileService } from '../../../../../platform/files/common/files.js';
import type { IWorkspaceContextService } from '../../../../../platform/workspace/common/workspace.js';

function makeWorkspaceContextService(workspaceUri: URI) {
	return {
		getWorkspace() {
			return {
				folders: [{
					uri: workspaceUri,
					name: 'test-workspace',
					index: 0,
					toResource(relativePath: string) {
						return URI.joinPath(workspaceUri, relativePath);
					},
				}],
			};
		},
		getWorkbenchState() { return 2; /* FOLDER */ },
		getWorkspaceFolder() { return undefined; },
		isInsideWorkspace() { return true; },
		isCurrentWorkspace() { return true; },
		onDidChangeWorkspaceName: Event.None,
		onDidChangeWorkspaceFolders: Event.None,
		onDidChangeWorkbenchState: Event.None,
		onWillChangeWorkspaceFolders: Event.None,
	};
}

function makeEnvironmentService(): Partial<IEnvironmentService> {
	return {
		userRoamingDataHome: URI.file('/test-roaming'),
	};
}

suite('ForgeConfigService', () => {

	let disposables: DisposableStore;
	let fileService: IFileService;
	let workspaceUri: URI;

	setup(() => {
		disposables = new DisposableStore();
		fileService = disposables.add(new FileService(new NullLogService()));
		const fsProvider = disposables.add(new InMemoryFileSystemProvider());
		disposables.add(fileService.registerProvider(Schemas.file, fsProvider));
		workspaceUri = URI.file('/test-workspace');
	});

	teardown(() => {
		disposables.dispose();
	});

	ensureNoDisposablesAreLeakedInTestSuite();

	function createService(): ForgeConfigService {
		const contextService = makeWorkspaceContextService(workspaceUri);
		const environmentService = makeEnvironmentService();
		return disposables.add(new ForgeConfigService(
			fileService,
			contextService as unknown as IWorkspaceContextService,
			new NullLogService(),
			environmentService as IEnvironmentService,
		));
	}

	test('getConfig returns default config when no forge.json exists', () => {
		const service = createService();

		const config = service.getConfig();

		assert.strictEqual(config.defaultProvider, '');
		assert.strictEqual(config.defaultModel, '');
		assert.strictEqual(config.stream, true);
		assert.deepStrictEqual(config.providers, []);
	});

	test('getConfig returns parsed config from forge.json', async () => {
		const forgeConfig: ForgeConfig = {
			defaultProvider: 'openai',
			defaultModel: 'gpt-4o',
			providers: [
				{ name: 'openai', models: [{ id: 'gpt-4o', maxTokens: 2048 }] },
			],
		};
		const configUri = URI.joinPath(workspaceUri, 'forge.json');
		await fileService.writeFile(configUri, VSBuffer.fromString(JSON.stringify(forgeConfig)));

		const service = createService();

		// Wait for async config loading via the change event
		await Event.toPromise(service.onDidChange);

		const config = service.getConfig();

		assert.strictEqual(config.defaultProvider, 'openai');
		assert.strictEqual(config.defaultModel, 'gpt-4o');
		assert.strictEqual(config.providers.length, 1);
		assert.strictEqual(config.providers[0].models[0].maxTokens, 2048);
	});

	test('getConfig returns default config when forge.json has invalid JSON', async () => {
		const configUri = URI.joinPath(workspaceUri, 'forge.json');
		await fileService.writeFile(configUri, VSBuffer.fromString('{ not valid json ,,, }'));

		const service = createService();

		// Invalid JSON falls back to defaults — no change event fires since config stays as defaults
		await new Promise<void>(resolve => setTimeout(resolve, 150));

		const config = service.getConfig();

		// Should gracefully fall back to defaults, not throw
		assert.strictEqual(config.defaultProvider, '');
	});

	test('updateConfig merges partial config and fires onDidChange', async () => {
		const service = createService();

		const changes: ForgeConfig[] = [];
		disposables.add(service.onDidChange(config => changes.push(config)));

		await service.updateConfig({ defaultModel: 'gpt-4o-mini' });

		assert.ok(changes.length >= 1, 'onDidChange should fire');
		const lastConfig = changes[changes.length - 1];
		assert.strictEqual(lastConfig.defaultModel, 'gpt-4o-mini');
		assert.strictEqual(lastConfig.defaultProvider, ''); // default preserved
	});

	// --- File watcher and debounce ---

	test('file watcher triggers reload on external edit', async () => {
		// Write initial config
		const configUri = URI.joinPath(workspaceUri, 'forge.json');
		const initialConfig: ForgeConfig = {
			defaultProvider: 'anthropic',
			providers: [{ name: 'anthropic', models: [{ id: 'claude-sonnet-4-6' }] }],
		};
		await fileService.writeFile(configUri, VSBuffer.fromString(JSON.stringify(initialConfig)));

		const service = createService();
		await Event.toPromise(service.onDidChange);

		assert.strictEqual(service.getConfig().defaultProvider, 'anthropic');

		// Simulate external edit — change defaultProvider
		const updatedConfig: ForgeConfig = {
			defaultProvider: 'openai',
			providers: [{ name: 'openai', models: [{ id: 'gpt-4o' }] }],
		};
		await fileService.writeFile(configUri, VSBuffer.fromString(JSON.stringify(updatedConfig)));

		// Wait for the debounced file watcher to pick up the change
		await Event.toPromise(service.onDidChange);

		assert.strictEqual(service.getConfig().defaultProvider, 'openai');
		assert.strictEqual(service.getConfig().providers[0].name, 'openai');
	});

	test('rapid config changes debounce to single reload', async () => {
		const configUri = URI.joinPath(workspaceUri, 'forge.json');
		const initialConfig: ForgeConfig = {
			defaultProvider: 'v0',
			providers: [],
		};
		await fileService.writeFile(configUri, VSBuffer.fromString(JSON.stringify(initialConfig)));

		const service = createService();
		await Event.toPromise(service.onDidChange);

		const changeEvents: ForgeConfig[] = [];
		disposables.add(service.onDidChange(config => changeEvents.push(config)));

		// Write 5 times in rapid succession (within 100ms debounce window)
		for (let i = 1; i <= 5; i++) {
			await fileService.writeFile(configUri, VSBuffer.fromString(JSON.stringify({
				defaultProvider: `v${i}`,
				providers: [],
			})));
		}

		// Wait for debounce to settle (100ms debounce + margin)
		await new Promise<void>(resolve => setTimeout(resolve, 250));

		// Should have debounced — fewer events than writes
		assert.ok(changeEvents.length < 5, `expected fewer than 5 change events due to debounce, got ${changeEvents.length}`);
		assert.ok(changeEvents.length >= 1, 'should have at least 1 change event');

		// Final state should reflect the last write
		assert.strictEqual(service.getConfig().defaultProvider, 'v5');
	});

	// --- Phase 3.5: New config shape tests ---

	suite('Phase 3.5 — multi-provider config shape', () => {

		test('new config with defaultProvider + providers array parses correctly', async () => {
			const newConfig: ForgeConfig = {
				defaultProvider: 'anthropic',
				defaultModel: 'claude-sonnet-4-6',
				stream: true,
				providers: [
					{
						name: 'anthropic',
						models: [
							{ id: 'claude-sonnet-4-6', maxTokens: 4096 },
						],
					},
				],
			};
			const configUri = URI.joinPath(workspaceUri, 'forge.json');
			await fileService.writeFile(configUri, VSBuffer.fromString(JSON.stringify(newConfig)));

			const service = createService();
			await Event.toPromise(service.onDidChange);

			const config = service.getConfig();

			assert.strictEqual(config.defaultProvider, 'anthropic');
			assert.ok(Array.isArray(config.providers), 'providers should be an array');
			assert.strictEqual(config.providers.length, 1);
			assert.strictEqual(config.providers[0].name, 'anthropic');
		});

		test('multiple providers are preserved in config', async () => {
			const newConfig: ForgeConfig = {
				defaultProvider: 'openai',
				providers: [
					{
						name: 'openai',
						models: [{ id: 'gpt-4o' }],
					},
					{
						name: 'anthropic',
						models: [{ id: 'claude-sonnet-4-6' }],
					},
				],
			};
			const configUri = URI.joinPath(workspaceUri, 'forge.json');
			await fileService.writeFile(configUri, VSBuffer.fromString(JSON.stringify(newConfig)));

			const service = createService();
			await Event.toPromise(service.onDidChange);

			const config = service.getConfig();

			assert.strictEqual(config.providers.length, 2);
			const anthropic = config.providers.find(p => p.name === 'anthropic');
			assert.ok(anthropic);
		});

		test('config with nonexistent defaultProvider is preserved as-is (no validation)', async () => {
			const configWithBadDefault = {
				defaultProvider: 'nonexistent',
				providers: [
					{
						name: 'anthropic',
						models: [{ id: 'claude-sonnet-4-6' }],
					},
				],
			};
			const configUri = URI.joinPath(workspaceUri, 'forge.json');
			await fileService.writeFile(configUri, VSBuffer.fromString(JSON.stringify(configWithBadDefault)));

			const service = createService();
			await Event.toPromise(service.onDidChange);

			const config = service.getConfig();
			// Config service does not validate — nonexistent default passes through
			assert.strictEqual(config.defaultProvider, 'nonexistent');
		});

		test('duplicate provider names are preserved as-is (no validation)', async () => {
			const configWithDupes = {
				defaultProvider: 'anthropic',
				providers: [
					{
						name: 'anthropic',
						models: [{ id: 'claude-sonnet-4-6' }],
					},
					{
						name: 'anthropic',
						models: [{ id: 'claude-opus-4-6' }],
					},
				],
			};
			const configUri = URI.joinPath(workspaceUri, 'forge.json');
			await fileService.writeFile(configUri, VSBuffer.fromString(JSON.stringify(configWithDupes)));

			const service = createService();
			await Event.toPromise(service.onDidChange);

			const config = service.getConfig();
			// Config service does not validate — duplicates pass through
			assert.strictEqual(config.providers.length, 2);
		});

		test('duplicate model ids within a provider are preserved as-is (no validation)', async () => {
			const configWithDupes = {
				defaultProvider: 'anthropic',
				providers: [
					{
						name: 'anthropic',
						models: [
							{ id: 'claude-sonnet-4-6' },
							{ id: 'claude-sonnet-4-6' },
						],
					},
				],
			};
			const configUri = URI.joinPath(workspaceUri, 'forge.json');
			await fileService.writeFile(configUri, VSBuffer.fromString(JSON.stringify(configWithDupes)));

			const service = createService();
			await Event.toPromise(service.onDidChange);

			const config = service.getConfig();
			// Config service does not validate — duplicates pass through
			assert.strictEqual(config.providers[0].models.length, 2);
		});

		test('empty providers array is preserved as-is (no validation)', async () => {
			const emptyConfig = {
				defaultProvider: 'anthropic',
				providers: [],
			};
			const configUri = URI.joinPath(workspaceUri, 'forge.json');
			await fileService.writeFile(configUri, VSBuffer.fromString(JSON.stringify(emptyConfig)));

			const service = createService();
			await Event.toPromise(service.onDidChange);

			const config = service.getConfig();
			// Config service does not validate — empty providers passes through
			assert.strictEqual(config.defaultProvider, 'anthropic');
			assert.deepStrictEqual(config.providers, []);
		});

		test('config with all optional fields set preserves them', async () => {
			const fullConfig: ForgeConfig = {
				defaultProvider: 'anthropic',
				defaultModel: 'claude-opus-4-6',
				stream: false,
				providers: [
					{
						name: 'anthropic',
						baseURL: 'https://proxy.internal',
						envKey: 'MY_ANTHROPIC_KEY',
						models: [
							{ id: 'claude-sonnet-4-6', maxTokens: 8192, contextBudget: 16000 },
							{ id: 'claude-opus-4-6' },
						],
					},
				],
			};
			const configUri = URI.joinPath(workspaceUri, 'forge.json');
			await fileService.writeFile(configUri, VSBuffer.fromString(JSON.stringify(fullConfig)));

			const service = createService();
			await Event.toPromise(service.onDidChange);

			const config = service.getConfig();

			assert.strictEqual(config.stream, false);
			assert.strictEqual(config.defaultModel, 'claude-opus-4-6');
			assert.strictEqual(config.providers[0].baseURL, 'https://proxy.internal');
			assert.strictEqual(config.providers[0].envKey, 'MY_ANTHROPIC_KEY');
			assert.strictEqual(config.providers[0].models[0].maxTokens, 8192);
			assert.strictEqual(config.providers[0].models[0].contextBudget, 16000);
		});

		test('minimal model with only id field parses successfully', async () => {
			const minimalConfig: ForgeConfig = {
				defaultProvider: 'openai',
				providers: [
					{
						name: 'openai',
						models: [{ id: 'gpt-4o' }],
					},
				],
			};
			const configUri = URI.joinPath(workspaceUri, 'forge.json');
			await fileService.writeFile(configUri, VSBuffer.fromString(JSON.stringify(minimalConfig)));

			const service = createService();
			await Event.toPromise(service.onDidChange);

			const config = service.getConfig();

			assert.strictEqual(config.providers[0].models[0].id, 'gpt-4o');
			assert.strictEqual(config.providers[0].models[0].maxTokens, undefined);
			assert.strictEqual(config.providers[0].models[0].contextBudget, undefined);
		});

		test('provider with empty models array is preserved as-is (no validation)', async () => {
			const configWithEmptyModels = {
				defaultProvider: 'anthropic',
				providers: [
					{
						name: 'anthropic',
						models: [],
					},
				],
			};
			const configUri = URI.joinPath(workspaceUri, 'forge.json');
			await fileService.writeFile(configUri, VSBuffer.fromString(JSON.stringify(configWithEmptyModels)));

			const service = createService();
			await Event.toPromise(service.onDidChange);

			const config = service.getConfig();
			// Config service does not validate — empty models passes through
			assert.strictEqual(config.providers.length, 1);
			assert.deepStrictEqual(config.providers[0].models, []);
		});
	});
});
