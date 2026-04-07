/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import * as assert from 'assert';
// eslint-disable-next-line local/code-import-patterns
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../../../base/test/common/utils.js';
import { StepMCP } from '../stepMCP.js';
import { IForgeOnboardingService, IEnvironmentDetectionResult } from '../../../../../../services/forge/common/forgeOnboardingService.js';

suite('StepMCP', () => {
	ensureNoDisposablesAreLeakedInTestSuite();

	let savedSelections: string[] = [];

	const mockService: IForgeOnboardingService = {
		_serviceBrand: undefined,
		onboardingComplete: false,
		detectEnvironment: async () => ({} as IEnvironmentDetectionResult),
		markComplete: () => { },
		reset: () => { },
		saveMCPSelections: async (servers: string[]) => { savedSelections = servers; },
		getMCPSelections: async () => [],
	};

	function createStep() {
		savedSelections = [];
		return new StepMCP(mockService);
	}

	const baseEnv: IEnvironmentDetectionResult = {
		hasVSCodeConfig: false,
		vscodeConfigPath: undefined,
		detectedApiKeys: {},
		ollamaRunning: false,
		lmStudioRunning: false,
		npxAvailable: false,
	};

	test('stepId is mcp', () => {
		assert.strictEqual(createStep().stepId, 'mcp');
	});

	test('validate always returns true', () => {
		assert.strictEqual(createStep().validate(), true);
	});

	test('default selections are filesystem and github', () => {
		const step = createStep();
		assert.deepStrictEqual(step.selectedServers, ['filesystem', 'github']);
	});

	test('onNext saves selected servers', async () => {
		const step = createStep();
		await step.onNext();
		assert.deepStrictEqual(savedSelections, ['filesystem', 'github']);
	});

	test('toggling a server off removes it from selectedServers', () => {
		const step = createStep();
		step.toggleServer('github');
		assert.deepStrictEqual(step.selectedServers, ['filesystem']);
	});

	test('toggling a server on adds it to selectedServers', () => {
		const step = createStep();
		step.toggleServer('github'); // off
		step.toggleServer('github'); // on again
		assert.deepStrictEqual(step.selectedServers, ['filesystem', 'github']);
	});

	test('onNext saves empty array when all deselected', async () => {
		const step = createStep();
		step.toggleServer('filesystem');
		step.toggleServer('github');
		await step.onNext();
		assert.deepStrictEqual(savedSelections, []);
	});

	test('render produces mcp list with 4 options', () => {
		const step = createStep();
		const container = document.createElement('div');
		step.render(container, baseEnv);
		const options = container.querySelectorAll('.forge-onboarding-mcp-option');
		assert.strictEqual(options.length, 4);
	});
});
