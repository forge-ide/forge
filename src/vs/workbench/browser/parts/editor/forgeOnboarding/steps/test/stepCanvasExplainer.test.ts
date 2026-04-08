import * as assert from 'assert';
// eslint-disable-next-line local/code-import-patterns
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../../../base/test/common/utils.js';
import { StepCanvasExplainer } from '../stepCanvasExplainer.js';

suite('StepCanvasExplainer', () => {
	ensureNoDisposablesAreLeakedInTestSuite();

	function createStep() {
		return new StepCanvasExplainer();
	}

	test('stepId is canvas', () => {
		assert.strictEqual(createStep().stepId, 'canvas');
	});

	test('validate always returns true', () => {
		assert.strictEqual(createStep().validate(), true);
	});

	test('onNext resolves without throwing', async () => {
		await assert.doesNotReject(() => createStep().onNext());
	});

	test('render produces a quad preview element', () => {
		const step = createStep();
		const container = document.createElement('div');
		step.render(container, {
			hasVSCodeConfig: false,
			vscodeConfigPath: undefined,
			detectedApiKeys: {},
			ollamaRunning: false,
			lmStudioRunning: false,
			npxAvailable: false,
		});
		assert.ok(container.querySelector('.forge-onboarding-quad-preview'), 'quad preview not found');
	});

	test('render creates exactly 4 pane elements inside the preview', () => {
		const step = createStep();
		const container = document.createElement('div');
		step.render(container, {
			hasVSCodeConfig: false,
			vscodeConfigPath: undefined,
			detectedApiKeys: {},
			ollamaRunning: false,
			lmStudioRunning: false,
			npxAvailable: false,
		});
		const preview = container.querySelector('.forge-onboarding-quad-preview');
		assert.ok(preview, 'quad preview not found');
		const panes = preview.querySelectorAll('.forge-onboarding-pane');
		assert.strictEqual(panes.length, 4, 'expected exactly 4 panes');
	});

	test('render produces a required tag with correct text', () => {
		const step = createStep();
		const container = document.createElement('div');
		step.render(container, {
			hasVSCodeConfig: false,
			vscodeConfigPath: undefined,
			detectedApiKeys: {},
			ollamaRunning: false,
			lmStudioRunning: false,
			npxAvailable: false,
		});
		const requiredTag = container.querySelector('.forge-onboarding-required-tag');
		assert.ok(requiredTag, 'required tag not found');
		assert.ok(requiredTag.textContent?.includes('Cannot be skipped'), 'required tag does not contain "Cannot be skipped"');
	});
});
