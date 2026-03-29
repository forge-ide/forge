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

	const disposables = new DisposableStore();
	let fileService: IFileService;
	let workspaceUri: URI;

	ensureNoDisposablesAreLeakedInTestSuite();

	setup(() => {
		fileService = disposables.add(new FileService(new NullLogService()));
		const fsProvider = disposables.add(new InMemoryFileSystemProvider());
		disposables.add(fileService.registerProvider(Schemas.file, fsProvider));
		workspaceUri = URI.file('/test-workspace');
	});

	teardown(() => {
		disposables.clear();
	});

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

		assert.strictEqual(config.provider, 'anthropic');
		assert.strictEqual(config.model, 'claude-sonnet-4-6');
	});

	test('getConfig returns parsed config from forge.json', async () => {
		const forgeConfig: ForgeConfig = {
			provider: 'openai',
			model: 'gpt-4o',
			maxTokens: 2048,
		};
		const configUri = URI.joinPath(workspaceUri, 'forge.json');
		await fileService.writeFile(configUri, VSBuffer.fromString(JSON.stringify(forgeConfig)));

		const service = createService();

		// Allow async config loading to complete
		await new Promise<void>(resolve => setTimeout(resolve, 200));

		const config = service.getConfig();

		assert.strictEqual(config.provider, 'openai');
		assert.strictEqual(config.model, 'gpt-4o');
		assert.strictEqual(config.maxTokens, 2048);
	});

	test('getConfig returns default config when forge.json has invalid JSON', async () => {
		const configUri = URI.joinPath(workspaceUri, 'forge.json');
		await fileService.writeFile(configUri, VSBuffer.fromString('{ not valid json ,,, }'));

		const service = createService();

		// Allow async config loading to complete
		await new Promise<void>(resolve => setTimeout(resolve, 200));

		const config = service.getConfig();

		// Should gracefully fall back to defaults, not throw
		assert.strictEqual(config.provider, 'anthropic');
	});

	test('updateConfig merges partial config and fires onDidChange', async () => {
		const service = createService();

		const changes: ForgeConfig[] = [];
		disposables.add(service.onDidChange(config => changes.push(config)));

		await service.updateConfig({ model: 'gpt-4o-mini' });

		assert.ok(changes.length >= 1, 'onDidChange should fire');
		const lastConfig = changes[changes.length - 1];
		assert.strictEqual(lastConfig.model, 'gpt-4o-mini');
		assert.strictEqual(lastConfig.provider, 'anthropic'); // default preserved
	});
});
