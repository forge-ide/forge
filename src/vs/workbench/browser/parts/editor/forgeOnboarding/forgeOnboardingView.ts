/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { clearNode } from '../../../../../base/browser/dom.js';
import { Disposable } from '../../../../../base/common/lifecycle.js';
import { IForgeOnboardingService, IEnvironmentDetectionResult } from '../../../../services/forge/common/forgeOnboardingService.js';
import { IInstantiationService } from '../../../../../platform/instantiation/common/instantiation.js';
import { ICommandService } from '../../../../../platform/commands/common/commands.js';
import { ILogService } from '../../../../../platform/log/common/log.js';
import { IEditorService } from '../../../../services/editor/common/editorService.js';
import { IEditorGroupsService } from '../../../../services/editor/common/editorGroupsService.js';
import { Step1Welcome } from './steps/step1Welcome.js';
import { Step2Import } from './steps/step2Import.js';
import { Step3Provider } from './steps/step3Provider.js';
import { Step4Ready, IStep4ReadyOptions } from './steps/step4Ready.js';
import { StepCanvasExplainer } from './steps/stepCanvasExplainer.js';
import { StepMCP } from './steps/stepMCP.js';
import './forgeOnboardingView.css';

export interface IOnboardingStep {
	readonly stepId: string;
	readonly title: string;
	readonly subtitle: string;
	render(container: HTMLElement, env: IEnvironmentDetectionResult): void;
	validate(): boolean;
	onNext(): Promise<void>;
}

export class ForgeOnboardingView extends Disposable {
	private readonly _rootEl: HTMLElement;
	private readonly _wizardEl: HTMLElement;
	private readonly _progressEl: HTMLElement;
	private readonly _contentEl: HTMLElement;
	private readonly _footerEl: HTMLElement;
	private readonly _prevBtn: HTMLButtonElement;
	private readonly _nextBtn: HTMLButtonElement;

	private _progressFillEl!: HTMLElement;
	private _stepCounterEl!: HTMLElement;

	private _env: IEnvironmentDetectionResult | undefined;
	private _state: WizardState = 'detecting';
	private _currentStep: IOnboardingStep | undefined;

	// Keep step instances alive so selections survive navigation
	private _step2: Step2Import | undefined;
	private _step3: Step3Provider | undefined;
	private _stepCanvas: StepCanvasExplainer | undefined;
	private _stepMCP: StepMCP | undefined;

	private _validationErrorEl: HTMLElement | undefined;

	constructor(
		container: HTMLElement,
		@IForgeOnboardingService private readonly onboardingService: IForgeOnboardingService,
		@IInstantiationService private readonly instantiationService: IInstantiationService,
		@ICommandService private readonly _commandService: ICommandService,
		@ILogService private readonly _logService: ILogService,
		@IEditorService private readonly _editorService: IEditorService,
		@IEditorGroupsService private readonly _editorGroupsService: IEditorGroupsService,
	) {
		super();

		// Root
		this._rootEl = document.createElement('div');
		this._rootEl.className = 'forge-onboarding-root';
		container.appendChild(this._rootEl);

		// Wizard card
		this._wizardEl = document.createElement('div');
		this._wizardEl.className = 'forge-onboarding-wizard';
		this._rootEl.appendChild(this._wizardEl);

		// Progress bar
		this._progressEl = document.createElement('div');
		this._progressEl.className = 'forge-onboarding-progress';

		const progressWrap = document.createElement('div');
		progressWrap.className = 'forge-onboarding-progress-wrap';

		this._stepCounterEl = document.createElement('div');
		this._stepCounterEl.className = 'forge-onboarding-step-counter';
		progressWrap.appendChild(this._stepCounterEl);

		const track = document.createElement('div');
		track.className = 'forge-onboarding-progress-track';

		this._progressFillEl = document.createElement('div');
		this._progressFillEl.className = 'forge-onboarding-progress-fill';
		track.appendChild(this._progressFillEl);
		progressWrap.appendChild(track);

		this._progressEl.appendChild(progressWrap);
		this._wizardEl.appendChild(this._progressEl);

		// Content area
		this._contentEl = document.createElement('div');
		this._contentEl.className = 'forge-onboarding-content';
		this._wizardEl.appendChild(this._contentEl);

		// Footer
		this._footerEl = document.createElement('div');
		this._footerEl.className = 'forge-onboarding-footer';

		this._prevBtn = document.createElement('button');
		this._prevBtn.className = 'forge-onboarding-btn-secondary';
		this._prevBtn.textContent = 'Back';

		this._nextBtn = document.createElement('button');
		this._nextBtn.className = 'forge-onboarding-btn-primary';
		this._nextBtn.textContent = 'Next';
		this._nextBtn.addEventListener('click', () => this._onNext());

		this._footerEl.appendChild(this._prevBtn);
		this._footerEl.appendChild(this._nextBtn);
		this._wizardEl.appendChild(this._footerEl);

		// Start in detecting state
		this._setState('detecting');
		this._detect();
	}

	layout(dimension: { width: number; height: number }): void {
		this._rootEl.style.width = `${dimension.width}px`;
		this._rootEl.style.height = `${dimension.height}px`;
	}

	private _setState(state: WizardState): void {
		this._state = state;
		const isDetecting = state === 'detecting';
		this._progressEl.style.display = isDetecting ? 'none' : 'flex';
		this._footerEl.style.display = isDetecting ? 'none' : 'flex';
		this._updateProgress(state);
	}

	private _updateProgress(state: WizardState): void {
		if (state === 'detecting' || state === 'complete') {
			this._stepCounterEl.textContent = '';
			this._progressFillEl.style.width = state === 'complete' ? '100%' : '0%';
			return;
		}
		const hasImport = this._env?.hasVSCodeConfig ?? false;
		const step = getStepNumber(state, hasImport);
		const total = getTotalSteps(hasImport);
		this._stepCounterEl.textContent = `Step ${step} of ${total}`;
		this._progressFillEl.style.width = `${(step / total) * 100}%`;
	}

	private async _detect(): Promise<void> {
		clearNode(this._contentEl);
		const loading = document.createElement('p');
		loading.className = 'forge-onboarding-loading';
		loading.textContent = 'Detecting your environment\u2026';
		this._contentEl.appendChild(loading);

		this._env = await this.onboardingService.detectEnvironment();
		this._setState('welcome');
		this._renderCurrentStep();
	}

	private _renderCurrentStep(): void {
		const el = this._contentEl;
		while (el.firstChild) { el.removeChild(el.firstChild); }

		switch (this._state) {
			case 'welcome': {
				const step = new Step1Welcome();
				step.render(this._contentEl, this._env!);
				this._currentStep = step;
				this._updateFooter('welcome');
				break;
			}
			case 'canvas': {
				if (!this._stepCanvas) {
					this._stepCanvas = new StepCanvasExplainer();
				}
				this._stepCanvas.render(this._contentEl, this._env!);
				this._currentStep = this._stepCanvas;
				this._updateFooter('canvas');
				break;
			}
			case 'import': {
				if (!this._step2) {
					this._step2 = new Step2Import();
				}
				this._step2.render(this._contentEl, this._env!);
				this._currentStep = this._step2;
				this._updateFooter('import');
				break;
			}
			case 'provider': {
				if (!this._step3) {
					this._step3 = this.instantiationService.createInstance(Step3Provider);
				}
				this._step3.render(this._contentEl, this._env!);
				this._currentStep = this._step3;
				this._updateFooter('provider');
				break;
			}
			case 'mcp': {
				if (!this._stepMCP) {
					this._stepMCP = new StepMCP(this.onboardingService);
				}
				this._stepMCP.render(this._contentEl, this._env!);
				this._currentStep = this._stepMCP;
				this._updateFooter('mcp');
				break;
			}
			case 'complete': {
				const options: IStep4ReadyOptions = {
					configuredProviders: this._step3?.configuredProviders ?? [],
					importedConfig: !!(this._step2?.importTheme || this._step2?.importKeybindings || this._step2?.importExtensions || this._step2?.importGit),
					mcpSelections: this._stepMCP?.selectedServers ?? [],
					onLaunch: (action) => this._handleLaunch(action),
				};
				const step = new Step4Ready(options);
				step.render(this._contentEl, this._env!);
				this._currentStep = step;
				this._updateFooter('complete');
				break;
			}
		}
	}

	private _updateFooter(state: WizardState): void {
		// Left button (skip/back)
		switch (state) {
			case 'welcome':
				this._prevBtn.textContent = 'Skip setup';
				this._prevBtn.style.display = '';
				this._prevBtn.className = 'forge-onboarding-btn-secondary';
				this._prevBtn.onclick = () => this._skipAll();
				break;
			case 'canvas':
				this._prevBtn.textContent = 'Back';
				this._prevBtn.style.display = '';
				this._prevBtn.className = 'forge-onboarding-btn-secondary';
				this._prevBtn.onclick = () => this._onPrev();
				break;
			case 'import':
				this._prevBtn.textContent = 'Skip import';
				this._prevBtn.style.display = '';
				this._prevBtn.className = 'forge-onboarding-btn-secondary';
				this._prevBtn.onclick = () => { this._step2 = undefined; this._setState('provider'); this._renderCurrentStep(); };
				break;
			case 'provider':
				this._prevBtn.textContent = 'Skip for now';
				this._prevBtn.style.display = '';
				this._prevBtn.className = 'forge-onboarding-btn-secondary';
				this._prevBtn.onclick = () => { this._setState('mcp'); this._renderCurrentStep(); };
				break;
			case 'mcp':
				this._prevBtn.textContent = 'Skip';
				this._prevBtn.style.display = '';
				this._prevBtn.className = 'forge-onboarding-btn-secondary';
				this._prevBtn.onclick = () => { this._stepMCP = undefined; this._setState('complete'); this._renderCurrentStep(); };
				break;
			case 'complete':
				this._prevBtn.style.display = 'none';
				break;
		}

		// Right button (next/primary action)
		const nextLabels: Partial<Record<WizardState, string>> = {
			welcome: 'Get started',
			canvas: 'Got it',
			import: 'Import selected',
			provider: 'Connect',
			mcp: 'Enable selected',
			complete: 'Enter Forge',
		};
		if (state === 'complete') {
			this._nextBtn.style.display = 'none';
		} else {
			this._nextBtn.style.display = '';
			this._nextBtn.textContent = nextLabels[state] ?? 'Next';
		}
	}

	private async _onNext(): Promise<void> {
		this._clearValidationError();
		if (this._currentStep && !this._currentStep.validate()) {
			this._showValidationError('Please complete this step before continuing.');
			return;
		}
		if (this._currentStep) {
			await this._currentStep.onNext();
		}
		const hasImport = this._env?.hasVSCodeConfig ?? false;
		const next = getNextState(this._state as WizardState, hasImport);
		this._setState(next);
		this._renderCurrentStep();
	}

	private _onPrev(): void {
		this._clearValidationError();
		const hasImport = this._env?.hasVSCodeConfig ?? false;
		const prev = getPrevState(this._state as WizardState, hasImport);
		if (prev) {
			this._setState(prev);
			this._renderCurrentStep();
		}
	}

	private _showValidationError(message: string): void {
		this._clearValidationError();
		const err = document.createElement('p');
		err.className = 'forge-onboarding-error';
		err.textContent = message;
		this._validationErrorEl = err;
		this._footerEl.insertAdjacentElement('beforebegin', err);
	}

	private _clearValidationError(): void {
		this._validationErrorEl?.remove();
		this._validationErrorEl = undefined;
	}

	private async _closeOnboarding(): Promise<void> {
		const input = this._editorService.activeEditor;
		if (input) {
			await this._editorService.closeEditor({
				editor: input,
				groupId: this._editorGroupsService.activeGroup.id,
			});
		}
	}

	private _skipAll(): void {
		this.onboardingService.markComplete();
		void this._closeOnboarding();
	}

	private async _handleLaunch(action: 'openFolder' | 'newSession'): Promise<void> {
		this.onboardingService.markComplete();
		await this._closeOnboarding();
		try {
			if (action === 'openFolder') {
				await this._commandService.executeCommand('workbench.action.files.openFolder');
			} else {
				await this._commandService.executeCommand('forge.workspace.create');
			}
		} catch (err) {
			this._logService.error('ForgeOnboardingView: launch command failed', err);
		}
	}
}

export type WizardState = 'detecting' | 'welcome' | 'canvas' | 'import' | 'provider' | 'mcp' | 'complete';

export function getTotalSteps(hasVSCodeConfig: boolean): number {
	return hasVSCodeConfig ? 5 : 4;
}

export function getStepNumber(state: WizardState, hasVSCodeConfig: boolean): number {
	switch (state) {
		case 'welcome': return 1;
		case 'canvas': return 2;
		case 'import': return 3;
		case 'provider': return hasVSCodeConfig ? 4 : 3;
		case 'mcp': return hasVSCodeConfig ? 5 : 4;
		case 'complete': return getTotalSteps(hasVSCodeConfig);
		case 'detecting': return 0;
		default: return 0;
	}
}

export function getNextState(state: WizardState, hasVSCodeConfig: boolean): WizardState {
	switch (state) {
		case 'welcome': return 'canvas';
		case 'canvas': return hasVSCodeConfig ? 'import' : 'provider';
		case 'import': return 'provider';
		case 'provider': return 'mcp';
		case 'mcp': return 'complete';
		default: return 'complete';
	}
}

export function getPrevState(state: WizardState, hasVSCodeConfig: boolean): WizardState | null {
	switch (state) {
		case 'canvas': return 'welcome';
		case 'import': return 'canvas';
		case 'provider': return hasVSCodeConfig ? 'import' : 'canvas';
		case 'mcp': return 'provider';
		default: return null;
	}
}
