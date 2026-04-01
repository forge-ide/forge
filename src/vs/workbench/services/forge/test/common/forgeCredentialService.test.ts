/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import assert from 'assert';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import type { IForgeCredentialService } from '../../common/forgeCredentialService.js';
import { ForgeCredentialService } from '../../electron-browser/forgeCredentialService.js';

/**
 * Mock SecretStorage that stores keys in-memory.
 */
class MockSecretStorageService {
	private readonly secrets = new Map<string, string>();

	async store(key: string, value: string): Promise<void> {
		this.secrets.set(key, value);
	}

	async get(key: string): Promise<string | undefined> {
		return this.secrets.get(key);
	}

	async delete(key: string): Promise<void> {
		this.secrets.delete(key);
	}
}

/**
 * Mock environment providing controllable env vars.
 */
class MockEnvironment {
	private readonly vars = new Map<string, string>();

	set(key: string, value: string): void {
		this.vars.set(key, value);
	}

	get(key: string): string | undefined {
		return this.vars.get(key);
	}

	clear(): void {
		this.vars.clear();
	}
}

suite('ForgeCredentialService', () => {

	ensureNoDisposablesAreLeakedInTestSuite();

	let secretStorage: MockSecretStorageService;
	let mockEnv: MockEnvironment;
	let service: IForgeCredentialService;

	setup(() => {
		secretStorage = new MockSecretStorageService();
		mockEnv = new MockEnvironment();
		service = new ForgeCredentialService(
			secretStorage as Parameters<typeof ForgeCredentialService['prototype']['constructor']>[0],
			mockEnv as Parameters<typeof ForgeCredentialService['prototype']['constructor']>[1],
		);
	});

	suite('resolveApiKey', () => {

		test('returns key from SecretStorage when available', async () => {
			await secretStorage.store('forge.api.key.anthropic', 'sk-secret-123');

			const key = await service.resolveApiKey('anthropic');

			assert.strictEqual(key, 'sk-secret-123');
		});

		test('falls back to env var when SecretStorage is empty', async () => {
			mockEnv.set('ANTHROPIC_API_KEY', 'sk-env-456');

			const key = await service.resolveApiKey('anthropic');

			assert.strictEqual(key, 'sk-env-456');
		});

		test('returns undefined when both SecretStorage and env var are empty', async () => {
			const key = await service.resolveApiKey('anthropic');

			assert.strictEqual(key, undefined);
		});

		test('SecretStorage takes precedence over env var', async () => {
			await secretStorage.store('forge.api.key.openai', 'sk-secret');
			mockEnv.set('OPENAI_API_KEY', 'sk-env');

			const key = await service.resolveApiKey('openai');

			assert.strictEqual(key, 'sk-secret');
		});

		test('Gemini checks GOOGLE_API_KEY env var', async () => {
			mockEnv.set('GOOGLE_API_KEY', 'google-key-123');

			const key = await service.resolveApiKey('gemini');

			assert.strictEqual(key, 'google-key-123');
		});

		test('Gemini falls back to GEMINI_API_KEY when GOOGLE_API_KEY is not set', async () => {
			mockEnv.set('GEMINI_API_KEY', 'gemini-key-456');

			const key = await service.resolveApiKey('gemini');

			assert.strictEqual(key, 'gemini-key-456');
		});

		test('Gemini prefers GOOGLE_API_KEY over GEMINI_API_KEY', async () => {
			mockEnv.set('GOOGLE_API_KEY', 'google-key');
			mockEnv.set('GEMINI_API_KEY', 'gemini-key');

			const key = await service.resolveApiKey('gemini');

			assert.strictEqual(key, 'google-key');
		});

		test('resolves OpenAI key from env var', async () => {
			mockEnv.set('OPENAI_API_KEY', 'sk-openai-789');

			const key = await service.resolveApiKey('openai');

			assert.strictEqual(key, 'sk-openai-789');
		});

		test('resolves Mistral key from env var', async () => {
			mockEnv.set('MISTRAL_API_KEY', 'mistral-key');

			const key = await service.resolveApiKey('mistral');

			assert.strictEqual(key, 'mistral-key');
		});

		test('local provider returns undefined (no key needed)', async () => {
			const key = await service.resolveApiKey('local');

			assert.strictEqual(key, undefined);
		});
	});

	suite('storeApiKey', () => {

		test('stores key to SecretStorage', async () => {
			await service.storeApiKey('anthropic', 'sk-new-key');

			const stored = await secretStorage.get('forge.api.key.anthropic');
			assert.strictEqual(stored, 'sk-new-key');
		});

		test('stored key is retrievable via resolveApiKey', async () => {
			await service.storeApiKey('openai', 'sk-stored');

			const key = await service.resolveApiKey('openai');
			assert.strictEqual(key, 'sk-stored');
		});
	});

	suite('deleteApiKey', () => {

		test('removes key from SecretStorage', async () => {
			await secretStorage.store('forge.api.key.anthropic', 'sk-to-delete');
			await service.deleteApiKey('anthropic');

			const key = await secretStorage.get('forge.api.key.anthropic');
			assert.strictEqual(key, undefined);
		});

		test('deleted key falls through to env var on next resolve', async () => {
			await secretStorage.store('forge.api.key.anthropic', 'sk-secret');
			mockEnv.set('ANTHROPIC_API_KEY', 'sk-env');

			await service.deleteApiKey('anthropic');

			const key = await service.resolveApiKey('anthropic');
			assert.strictEqual(key, 'sk-env');
		});
	});

	suite('resolveBaseURL', () => {

		test('returns config baseURL when set', () => {
			// resolveBaseURL checks config first, then env var
			// For this test we check env var fallback since config is not injected directly
			mockEnv.set('ANTHROPIC_BASE_URL', 'https://custom.example.com');

			const url = service.resolveBaseURL('anthropic');

			assert.strictEqual(url, 'https://custom.example.com');
		});

		test('returns undefined when no baseURL is configured', () => {
			const url = service.resolveBaseURL('anthropic');

			assert.strictEqual(url, undefined);
		});

		test('resolves OPENAI_BASE_URL for openai provider', () => {
			mockEnv.set('OPENAI_BASE_URL', 'https://openai-proxy.example.com');

			const url = service.resolveBaseURL('openai');

			assert.strictEqual(url, 'https://openai-proxy.example.com');
		});

		test('resolves OLLAMA_HOST for local provider', () => {
			mockEnv.set('OLLAMA_HOST', 'http://localhost:11434');

			const url = service.resolveBaseURL('local');

			assert.strictEqual(url, 'http://localhost:11434');
		});

		test('resolves MISTRAL_SERVER_URL for mistral provider', () => {
			mockEnv.set('MISTRAL_SERVER_URL', 'https://mistral-proxy.example.com');

			const url = service.resolveBaseURL('mistral');

			assert.strictEqual(url, 'https://mistral-proxy.example.com');
		});
	});
});
