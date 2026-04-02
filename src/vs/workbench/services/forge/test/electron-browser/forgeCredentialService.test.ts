/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import assert from 'assert';
import { Emitter, Event } from '../../../../../base/common/event.js';
import { DisposableStore } from '../../../../../base/common/lifecycle.js';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import { NullLogService } from '../../../../../platform/log/common/log.js';
import type { ISecretStorageService } from '../../../../../platform/secrets/common/secrets.js';
import { ForgeCredentialService } from '../../electron-browser/forgeCredentialService.js';

/**
 * Mock SecretStorage that stores keys in-memory.
 * Implements the subset of ISecretStorageService used by ForgeCredentialService.
 */
class MockSecretStorageService {
	readonly _serviceBrand: undefined;
	readonly type = 'in-memory' as const;

	private readonly secrets = new Map<string, string>();
	private readonly _onDidChangeSecret = new Emitter<string>();
	readonly onDidChangeSecret: Event<string> = this._onDidChangeSecret.event;

	async set(key: string, value: string): Promise<void> {
		this.secrets.set(key, value);
		this._onDidChangeSecret.fire(key);
	}

	async get(key: string): Promise<string | undefined> {
		return this.secrets.get(key);
	}

	async delete(key: string): Promise<void> {
		this.secrets.delete(key);
		this._onDidChangeSecret.fire(key);
	}

	async keys(): Promise<string[]> {
		return Array.from(this.secrets.keys());
	}

	dispose(): void {
		this._onDidChangeSecret.dispose();
	}
}

suite('ForgeCredentialService', () => {

	let disposables: DisposableStore;
	let secretStorage: MockSecretStorageService;
	let service: ForgeCredentialService;

	setup(() => {
		disposables = new DisposableStore();
		secretStorage = new MockSecretStorageService();
		disposables.add({ dispose: () => secretStorage.dispose() });
		service = new ForgeCredentialService(
			secretStorage as unknown as ISecretStorageService,
			new NullLogService(),
		);
		disposables.add(service);
	});

	teardown(() => {
		disposables.dispose();
	});

	ensureNoDisposablesAreLeakedInTestSuite();

	suite('getApiKey', () => {

		test('returns key from SecretStorage when available', async () => {
			await secretStorage.set('forge.provider.anthropic', 'sk-secret-123');

			const key = await service.getApiKey('anthropic', 'ANTHROPIC_API_KEY');

			assert.strictEqual(key, 'sk-secret-123');
		});

		test('returns undefined when both SecretStorage and env var are empty', async () => {
			const key = await service.getApiKey('anthropic', 'NONEXISTENT_ENV_VAR_FOR_TEST');

			assert.strictEqual(key, undefined);
		});

		test('SecretStorage takes precedence over env var', async () => {
			await secretStorage.set('forge.provider.openai', 'sk-secret');

			const key = await service.getApiKey('openai', 'OPENAI_API_KEY');

			assert.strictEqual(key, 'sk-secret');
		});
	});

	suite('setApiKey', () => {

		test('stores key to SecretStorage', async () => {
			await service.setApiKey('anthropic', 'sk-new-key');

			const stored = await secretStorage.get('forge.provider.anthropic');
			assert.strictEqual(stored, 'sk-new-key');
		});

		test('stored key is retrievable via getApiKey', async () => {
			await service.setApiKey('openai', 'sk-stored');

			const key = await service.getApiKey('openai', 'OPENAI_API_KEY');
			assert.strictEqual(key, 'sk-stored');
		});
	});

	suite('deleteApiKey', () => {

		test('removes key from SecretStorage', async () => {
			await secretStorage.set('forge.provider.anthropic', 'sk-to-delete');
			await service.deleteApiKey('anthropic');

			const key = await secretStorage.get('forge.provider.anthropic');
			assert.strictEqual(key, undefined);
		});

		test('deleted key returns undefined on next getApiKey', async () => {
			await secretStorage.set('forge.provider.anthropic', 'sk-secret');
			await service.deleteApiKey('anthropic');

			const key = await service.getApiKey('anthropic', 'NONEXISTENT_ENV_VAR_FOR_TEST');
			assert.strictEqual(key, undefined);
		});
	});

	suite('hasApiKey', () => {

		test('returns true when key exists in SecretStorage', async () => {
			await secretStorage.set('forge.provider.anthropic', 'sk-123');

			const has = await service.hasApiKey('anthropic', 'ANTHROPIC_API_KEY');
			assert.strictEqual(has, true);
		});

		test('returns false when no key exists', async () => {
			const has = await service.hasApiKey('anthropic', 'NONEXISTENT_ENV_VAR_FOR_TEST');
			assert.strictEqual(has, false);
		});
	});

	suite('onDidChangeCredential', () => {

		test('fires provider name when secret changes', async () => {
			const changes: string[] = [];
			disposables.add(service.onDidChangeCredential(name => changes.push(name)));

			await service.setApiKey('anthropic', 'sk-new');

			assert.deepStrictEqual(changes, ['anthropic']);
		});

		test('fires provider name when key is deleted', async () => {
			await service.setApiKey('openai', 'sk-to-delete');

			const changes: string[] = [];
			disposables.add(service.onDidChangeCredential(name => changes.push(name)));

			await service.deleteApiKey('openai');

			assert.deepStrictEqual(changes, ['openai']);
		});

		test('does not fire for secrets outside the forge.provider. prefix', async () => {
			const changes: string[] = [];
			disposables.add(service.onDidChangeCredential(name => changes.push(name)));

			// Set a secret with a key that doesn't match the forge.provider. prefix
			await secretStorage.set('unrelated.secret.key', 'some-value');

			assert.deepStrictEqual(changes, [], 'should not fire for unrelated secrets');
		});
	});

	suite('overwrite and edge cases', () => {

		test('overwriting an existing key returns the newer value', async () => {
			await service.setApiKey('anthropic', 'sk-old');
			await service.setApiKey('anthropic', 'sk-new');

			const key = await service.getApiKey('anthropic', 'NONEXISTENT_ENV_VAR_FOR_TEST');
			assert.strictEqual(key, 'sk-new');
		});
	});

	suite('env var fallback', () => {

		const TEST_ENV_KEY = '__FORGE_TEST_ENV_KEY_' + Date.now();
		const CUSTOM_ENV_KEY = '__FORGE_CUSTOM_KEY_' + Date.now();

		teardown(() => {
			delete process.env[TEST_ENV_KEY];
			delete process.env[CUSTOM_ENV_KEY];
		});

		test('env var is used as fallback when SecretStorage is empty', async () => {
			process.env[TEST_ENV_KEY] = 'sk-from-env';

			const key = await service.getApiKey('testprovider', TEST_ENV_KEY);

			assert.strictEqual(key, 'sk-from-env');
		});

		test('custom envKey env var lookup works', async () => {
			process.env[CUSTOM_ENV_KEY] = 'sk-custom-env';

			const key = await service.getApiKey('anthropic', CUSTOM_ENV_KEY);

			assert.strictEqual(key, 'sk-custom-env');
		});

		test('hasApiKey returns true with env var only', async () => {
			process.env[TEST_ENV_KEY] = 'sk-from-env';

			const has = await service.hasApiKey('testprovider', TEST_ENV_KEY);

			assert.strictEqual(has, true);
		});
	});
});
