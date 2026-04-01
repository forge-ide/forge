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

		assert.strictEqual(config.defaultProvider, 'anthropic');
		assert.strictEqual(config.defaultModel, 'claude-sonnet-4-6');
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
		assert.strictEqual(config.defaultProvider, 'anthropic');
	});

	test('updateConfig merges partial config and fires onDidChange', async () => {
		const service = createService();

		const changes: ForgeConfig[] = [];
		disposables.add(service.onDidChange(config => changes.push(config)));

		await service.updateConfig({ defaultModel: 'gpt-4o-mini' });

		assert.ok(changes.length >= 1, 'onDidChange should fire');
		const lastConfig = changes[changes.length - 1];
		assert.strictEqual(lastConfig.defaultModel, 'gpt-4o-mini');
		assert.strictEqual(lastConfig.defaultProvider, 'anthropic'); // default preserved
	});

	// --- Phase 3.5: New config shape tests ---

	suite('Phase 3.5 — multi-provider config shape', () => {

		test('new config with default + providers array parses correctly', async () => {
			const newConfig = {
				default: 'anthropic',
				providers: [
					{
						name: 'anthropic',
						default: 'claude-sonnet-4-6',
						stream: true,
						models: [
							{ name: 'claude-sonnet-4-6', maxTokens: 4096 },
						],
					},
				],
			};
			const configUri = URI.joinPath(workspaceUri, 'forge.json');
			await fileService.writeFile(configUri, VSBuffer.fromString(JSON.stringify(newConfig)));

			const service = createService();
			await Event.toPromise(service.onDidChange);

			const config = service.getConfig();

			assert.strictEqual(config.default, 'anthropic');
			assert.ok(Array.isArray(config.providers), 'providers should be an array');
			assert.strictEqual(config.providers!.length, 1);
			assert.strictEqual(config.providers![0].name, 'anthropic');
		});

		test('enabled: false provider is preserved in config', async () => {
			const newConfig = {
				default: 'openai',
				providers: [
					{
						name: 'openai',
						default: 'gpt-4o',
						models: [{ name: 'gpt-4o' }],
					},
					{
						name: 'gemini',
						default: 'gemini-2.0-flash',
						enabled: false,
						models: [{ name: 'gemini-2.0-flash' }],
					},
				],
			};
			const configUri = URI.joinPath(workspaceUri, 'forge.json');
			await fileService.writeFile(configUri, VSBuffer.fromString(JSON.stringify(newConfig)));

			const service = createService();
			await Event.toPromise(service.onDidChange);

			const config = service.getConfig();

			assert.strictEqual(config.providers!.length, 2);
			const gemini = config.providers!.find(p => p.name === 'gemini');
			assert.ok(gemini);
			assert.strictEqual(gemini.enabled, false);
		});

		test('validation: config.default must match a provider name', async () => {
			const invalidConfig = {
				default: 'nonexistent',
				providers: [
					{
						name: 'anthropic',
						default: 'claude-sonnet-4-6',
						models: [{ name: 'claude-sonnet-4-6' }],
					},
				],
			};
			const configUri = URI.joinPath(workspaceUri, 'forge.json');
			await fileService.writeFile(configUri, VSBuffer.fromString(JSON.stringify(invalidConfig)));

			const service = createService();

			// Invalid config should fall back to defaults or report validation error
			await new Promise<void>(resolve => setTimeout(resolve, 150));

			const config = service.getConfig();
			// The service should reject or fall back — default should not be 'nonexistent'
			// (exact behavior depends on implementation: fallback to defaults or throw)
			assert.ok(
				config.default !== 'nonexistent' || config.provider === 'anthropic',
				'Invalid default should be rejected or fall back'
			);
		});

		test('validation: provider.default must match a model name', async () => {
			const invalidConfig = {
				default: 'anthropic',
				providers: [
					{
						name: 'anthropic',
						default: 'nonexistent-model',
						models: [{ name: 'claude-sonnet-4-6' }],
					},
				],
			};
			const configUri = URI.joinPath(workspaceUri, 'forge.json');
			await fileService.writeFile(configUri, VSBuffer.fromString(JSON.stringify(invalidConfig)));

			const service = createService();

			await new Promise<void>(resolve => setTimeout(resolve, 150));

			const config = service.getConfig();
			// Provider default 'nonexistent-model' does not match any model name
			// Implementation should reject or fall back
			if (config.providers && config.providers.length > 0) {
				const provider = config.providers[0];
				assert.ok(
					provider.default !== 'nonexistent-model' || config.provider === 'anthropic',
					'Invalid provider default should be rejected or fall back'
				);
			}
		});

		test('validation: no duplicate provider names', async () => {
			const invalidConfig = {
				default: 'anthropic',
				providers: [
					{
						name: 'anthropic',
						default: 'claude-sonnet-4-6',
						models: [{ name: 'claude-sonnet-4-6' }],
					},
					{
						name: 'anthropic',
						default: 'claude-opus-4-6',
						models: [{ name: 'claude-opus-4-6' }],
					},
				],
			};
			const configUri = URI.joinPath(workspaceUri, 'forge.json');
			await fileService.writeFile(configUri, VSBuffer.fromString(JSON.stringify(invalidConfig)));

			const service = createService();

			await new Promise<void>(resolve => setTimeout(resolve, 150));

			const config = service.getConfig();
			// Duplicate provider names should be rejected
			if (config.providers) {
				const names = config.providers.map(p => p.name);
				const uniqueNames = new Set(names);
				assert.strictEqual(names.length, uniqueNames.size, 'Duplicate provider names should be rejected');
			}
		});

		test('validation: no duplicate model names within a provider', async () => {
			const invalidConfig = {
				default: 'anthropic',
				providers: [
					{
						name: 'anthropic',
						default: 'claude-sonnet-4-6',
						models: [
							{ name: 'claude-sonnet-4-6' },
							{ name: 'claude-sonnet-4-6' },
						],
					},
				],
			};
			const configUri = URI.joinPath(workspaceUri, 'forge.json');
			await fileService.writeFile(configUri, VSBuffer.fromString(JSON.stringify(invalidConfig)));

			const service = createService();

			await new Promise<void>(resolve => setTimeout(resolve, 150));

			const config = service.getConfig();
			// Duplicate model names should be rejected
			if (config.providers && config.providers.length > 0) {
				const models = config.providers[0].models;
				if (models) {
					const names = models.map(m => m.name);
					const uniqueNames = new Set(names);
					assert.strictEqual(names.length, uniqueNames.size, 'Duplicate model names should be rejected');
				}
			}
		});

		test('validation: at least one provider required', async () => {
			const invalidConfig = {
				default: 'anthropic',
				providers: [],
			};
			const configUri = URI.joinPath(workspaceUri, 'forge.json');
			await fileService.writeFile(configUri, VSBuffer.fromString(JSON.stringify(invalidConfig)));

			const service = createService();

			await new Promise<void>(resolve => setTimeout(resolve, 150));

			const config = service.getConfig();
			// Empty providers array should be rejected — should fall back to defaults
			assert.ok(
				!config.providers || config.providers.length > 0 || config.provider === 'anthropic',
				'Empty providers array should be rejected or fall back to defaults'
			);
		});

		test('validation: at least one model per provider required', async () => {
			const invalidConfig = {
				default: 'anthropic',
				providers: [
					{
						name: 'anthropic',
						default: 'claude-sonnet-4-6',
						models: [],
					},
				],
			};
			const configUri = URI.joinPath(workspaceUri, 'forge.json');
			await fileService.writeFile(configUri, VSBuffer.fromString(JSON.stringify(invalidConfig)));

			const service = createService();

			await new Promise<void>(resolve => setTimeout(resolve, 150));

			const config = service.getConfig();
			// Provider with empty models should be rejected
			if (config.providers && config.providers.length > 0) {
				assert.ok(
					config.providers[0].models === undefined || config.providers[0].models.length > 0,
					'Provider with no models should be rejected or removed'
				);
			}
		});
	});
});
