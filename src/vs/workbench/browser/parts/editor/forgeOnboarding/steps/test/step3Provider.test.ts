/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import * as assert from 'assert';
// eslint-disable-next-line local/code-import-patterns
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../../../base/test/common/utils.js';
import { IEnvironmentDetectionResult } from '../../../../../../services/forge/common/forgeOnboardingService.js';
import type { ISecretStorageService } from '../../../../../../../platform/secrets/common/secrets.js';
import type { IConfigurationService } from '../../../../../../../platform/configuration/common/configuration.js';
import type { IFileDialogService } from '../../../../../../../platform/dialogs/common/dialogs.js';
import type { IFileService } from '../../../../../../../platform/files/common/files.js';
import type { ILogService } from '../../../../../../../platform/log/common/log.js';
import { Step3Provider } from '../step3Provider.js';

const baseEnv: IEnvironmentDetectionResult = {
	hasVSCodeConfig: false,
	vscodeConfigPath: undefined,
	detectedApiKeys: {},
	ollamaRunning: false,
	lmStudioRunning: false,
	npxAvailable: false,
	vertexEnv: { projectId: undefined, location: undefined },
};

function createStep() {
	const storedSecrets = new Map<string, string>();
	let savedConfig: unknown;

	const mockSecretStorage = {
		get: async (key: string) => storedSecrets.get(key),
		set: async (key: string, value: string) => { storedSecrets.set(key, value); },
		delete: async (key: string) => { storedSecrets.delete(key); },
		onDidChange: { event: () => ({ dispose: () => { } }) },
	};

	const mockConfigService = {
		getValue: (_key: string) => undefined as unknown,
		updateValue: async (_key: string, value: unknown) => { savedConfig = value; },
		onDidChangeConfiguration: { event: () => ({ dispose: () => { } }) },
	};

	const mockFileDialogService = {
		showOpenDialog: async () => undefined,
	};

	const mockFileService = {
		readFile: async () => ({ value: { toString: () => '' } }),
	};

	const mockLogService = {
		trace: () => { },
		debug: () => { },
		info: () => { },
		warn: () => { },
		error: () => { },
	};

	const step = new Step3Provider(
		mockSecretStorage as unknown as ISecretStorageService,
		mockConfigService as unknown as IConfigurationService,
		mockFileDialogService as unknown as IFileDialogService,
		mockFileService as unknown as IFileService,
		mockLogService as unknown as ILogService,
	);

	return { step, storedSecrets, getSavedConfig: () => savedConfig };
}

suite('Step3Provider', () => {
	ensureNoDisposablesAreLeakedInTestSuite();

	suite('validate()', () => {
		test('returns false when no providers are selected', () => {
			const { step } = createStep();
			const container = document.createElement('div');
			step.render(container, baseEnv);
			assert.strictEqual(step.validate(), false);
		});

		test('returns false when Vertex is selected but projectId is empty', () => {
			const { step } = createStep();
			const container = document.createElement('div');
			step.render(container, baseEnv);

			const checkbox = container.querySelector<HTMLInputElement>('#provider-vertex');
			assert.ok(checkbox, 'Vertex checkbox must exist');
			checkbox.click();

			// Fill location but not projectId
			const inputs = container.querySelectorAll<HTMLInputElement>('.forge-onboarding-provider-field-input');
			const locationInput = Array.from(inputs).find(el => el.placeholder === 'us-central1');
			if (locationInput) { locationInput.value = 'us-central1'; locationInput.dispatchEvent(new Event('input')); }

			assert.strictEqual(step.validate(), false);
		});

		test('returns false when Vertex is selected but location is empty', () => {
			const { step } = createStep();
			const container = document.createElement('div');
			step.render(container, baseEnv);

			const checkbox = container.querySelector<HTMLInputElement>('#provider-vertex');
			assert.ok(checkbox, 'Vertex checkbox must exist');
			checkbox.click();

			const inputs = container.querySelectorAll<HTMLInputElement>('.forge-onboarding-provider-field-input');
			const projectInput = inputs[0];
			if (projectInput) { projectInput.value = 'my-project'; projectInput.dispatchEvent(new Event('input')); }

			assert.strictEqual(step.validate(), false);
		});

		test('returns true when Vertex is selected with projectId and location', () => {
			const { step } = createStep();
			const container = document.createElement('div');
			step.render(container, baseEnv);

			const checkbox = container.querySelector<HTMLInputElement>('#provider-vertex');
			assert.ok(checkbox, 'Vertex checkbox must exist');
			checkbox.click();

			const inputs = container.querySelectorAll<HTMLInputElement>('.forge-onboarding-provider-field-input');
			inputs[0].value = 'my-project'; inputs[0].dispatchEvent(new Event('input'));
			inputs[1].value = 'us-central1'; inputs[1].dispatchEvent(new Event('input'));

			assert.strictEqual(step.validate(), true);
		});

		test('returns true when Vertex has projectId+location even without serviceAccountJson', () => {
			const { step } = createStep();
			const container = document.createElement('div');
			step.render(container, baseEnv);

			const checkbox = container.querySelector<HTMLInputElement>('#provider-vertex');
			assert.ok(checkbox);
			checkbox.click();

			const inputs = container.querySelectorAll<HTMLInputElement>('.forge-onboarding-provider-field-input');
			inputs[0].value = 'proj'; inputs[0].dispatchEvent(new Event('input'));
			inputs[1].value = 'us-east1'; inputs[1].dispatchEvent(new Event('input'));

			assert.strictEqual(step.validate(), true);
		});

		test('returns true for legacy Anthropic provider with API key', () => {
			const { step } = createStep();
			const container = document.createElement('div');
			step.render(container, baseEnv);

			const checkbox = container.querySelector<HTMLInputElement>('#provider-anthropic');
			assert.ok(checkbox);
			checkbox.click();

			const keyInput = container.querySelector<HTMLInputElement>('.forge-onboarding-api-key-input');
			assert.ok(keyInput);
			keyInput.value = 'sk-ant-test';
			keyInput.dispatchEvent(new Event('input'));

			assert.strictEqual(step.validate(), true);
		});
	});

	suite('onNext()', () => {
		test('saves projectId and location to configuration service for Vertex', async () => {
			const { step, getSavedConfig } = createStep();
			const container = document.createElement('div');
			step.render(container, baseEnv);

			const checkbox = container.querySelector<HTMLInputElement>('#provider-vertex');
			assert.ok(checkbox);
			checkbox.click();

			const inputs = container.querySelectorAll<HTMLInputElement>('.forge-onboarding-provider-field-input');
			inputs[0].value = 'my-gcp-project'; inputs[0].dispatchEvent(new Event('input'));
			inputs[1].value = 'us-central1'; inputs[1].dispatchEvent(new Event('input'));

			await step.onNext();

			const saved = getSavedConfig() as Array<{ name: string; projectId: string; location: string }>;
			assert.ok(Array.isArray(saved), 'forge.providers must be an array');
			const vertex = saved.find(p => p.name === 'vertex');
			assert.ok(vertex, 'vertex entry must exist in saved config');
			assert.strictEqual(vertex.projectId, 'my-gcp-project');
			assert.strictEqual(vertex.location, 'us-central1');
		});

		test('saves serviceAccountJson to SecretStorage for Vertex', async () => {
			const { step, storedSecrets } = createStep();
			const container = document.createElement('div');
			step.render(container, baseEnv);

			const checkbox = container.querySelector<HTMLInputElement>('#provider-vertex');
			assert.ok(checkbox);
			checkbox.click();

			const inputs = container.querySelectorAll<HTMLInputElement>('.forge-onboarding-provider-field-input');
			inputs[0].value = 'proj'; inputs[0].dispatchEvent(new Event('input'));
			inputs[1].value = 'us-central1'; inputs[1].dispatchEvent(new Event('input'));

			const textarea = container.querySelector<HTMLTextAreaElement>('.forge-onboarding-provider-json-input');
			assert.ok(textarea, 'JSON textarea must exist');
			textarea.value = '{"type":"service_account"}';
			textarea.dispatchEvent(new Event('input'));

			await step.onNext();

			assert.strictEqual(storedSecrets.get('forge.provider.apiKey.vertex'), '{"type":"service_account"}');
		});

		test('saves empty string to SecretStorage when serviceAccountJson is blank', async () => {
			const { step, storedSecrets } = createStep();
			const container = document.createElement('div');
			step.render(container, baseEnv);

			const checkbox = container.querySelector<HTMLInputElement>('#provider-vertex');
			assert.ok(checkbox);
			checkbox.click();

			const inputs = container.querySelectorAll<HTMLInputElement>('.forge-onboarding-provider-field-input');
			inputs[0].value = 'proj'; inputs[0].dispatchEvent(new Event('input'));
			inputs[1].value = 'us-central1'; inputs[1].dispatchEvent(new Event('input'));

			await step.onNext();

			assert.strictEqual(storedSecrets.get('forge.provider.apiKey.vertex'), '');
		});

		test('still saves legacy Anthropic API key to SecretStorage', async () => {
			const { step, storedSecrets } = createStep();
			const container = document.createElement('div');
			step.render(container, baseEnv);

			const checkbox = container.querySelector<HTMLInputElement>('#provider-anthropic');
			assert.ok(checkbox);
			checkbox.click();

			const keyInput = container.querySelector<HTMLInputElement>('.forge-onboarding-api-key-input');
			assert.ok(keyInput);
			keyInput.value = 'sk-ant-test-key';
			keyInput.dispatchEvent(new Event('input'));

			await step.onNext();

			assert.strictEqual(storedSecrets.get('forge.provider.apiKey.anthropic'), 'sk-ant-test-key');
		});
	});
});
