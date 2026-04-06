/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { clearNode } from '../../../../../base/browser/dom.js';
import { Disposable } from '../../../../../base/common/lifecycle.js';
import { IForgeOnboardingService, IEnvironmentDetectionResult } from '../../../../services/forge/common/forgeOnboardingService.js';
import { IInstantiationService } from '../../../../../platform/instantiation/common/instantiation.js';
import { Step1Welcome } from './steps/step1Welcome.js';
import { Step2Import } from './steps/step2Import.js';
import { Step3Provider } from './steps/step3Provider.js';
import { Step4Ready, IStep4ReadyOptions } from './steps/step4Ready.js';
import './forgeOnboardingView.css';

export interface IOnboardingStep {
	readonly stepId: string;
	readonly title: string;
	readonly subtitle: string;
	render(container: HTMLElement, env: IEnvironmentDetectionResult): void;
	validate(): boolean;
	onNext(): Promise<void>;
}

type WizardState = 'detecting' | 'step1' | 'step2' | 'step3' | 'step4' | 'complete';

const TOTAL_STEPS = 4;

export class ForgeOnboardingView extends Disposable {
	private readonly _rootEl: HTMLElement;
	private readonly _wizardEl: HTMLElement;
	private readonly _progressEl: HTMLElement;
	private readonly _contentEl: HTMLElement;
	private readonly _footerEl: HTMLElement;
	private readonly _prevBtn: HTMLButtonElement;
	private readonly _nextBtn: HTMLButtonElement;
	private readonly _dots: HTMLElement[] = [];

	private _env: IEnvironmentDetectionResult | undefined;
	private _currentStepIndex = 0; // 0-based, maps to steps 1-4
	private _currentStep: IOnboardingStep | undefined;

	// Keep step instances alive so import selections survive navigation
	private _step2: Step2Import | undefined;
	private _step3: Step3Provider | undefined;

	private _validationErrorEl: HTMLElement | undefined;

	constructor(
		container: HTMLElement,
		@IForgeOnboardingService private readonly onboardingService: IForgeOnboardingService,
		@IInstantiationService private readonly instantiationService: IInstantiationService,
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

		// Progress dots
		this._progressEl = document.createElement('div');
		this._progressEl.className = 'forge-onboarding-progress';
		for (let i = 0; i < TOTAL_STEPS; i++) {
			const dot = document.createElement('div');
			dot.className = 'forge-onboarding-dot';
			this._dots.push(dot);
			this._progressEl.appendChild(dot);
		}
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
		this._prevBtn.addEventListener('click', () => this._onPrev());

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
		const isDetecting = state === 'detecting';
		this._progressEl.style.display = isDetecting ? 'none' : 'flex';
		this._footerEl.style.display = isDetecting ? 'none' : 'flex';
	}

	private async _detect(): Promise<void> {
		clearNode(this._contentEl);
		const loading = document.createElement('p');
		loading.className = 'forge-onboarding-loading';
		loading.textContent = 'Detecting your environment\u2026';
		this._contentEl.appendChild(loading);

		this._env = await this.onboardingService.detectEnvironment();
		this._renderStep(1);
	}

	private _renderStep(step: 1 | 2 | 3 | 4): void {
		this._currentStepIndex = step - 1;
		this._setState(`step${step}` as WizardState);

		// Update dots
		for (let i = 0; i < TOTAL_STEPS; i++) {
			this._dots[i].classList.toggle('active', i === this._currentStepIndex);
		}

		// Nav button states
		this._prevBtn.disabled = step === 1;
		this._nextBtn.textContent = step === TOTAL_STEPS ? 'Open Forge' : 'Next';

		// Build step instance
		const env = this._env!;
		let stepInstance: IOnboardingStep;

		switch (step) {
			case 1:
				stepInstance = new Step1Welcome();
				break;
			case 2:
				if (!this._step2) {
					this._step2 = new Step2Import();
				}
				stepInstance = this._step2;
				break;
			case 3:
				if (!this._step3) {
					this._step3 = this.instantiationService.createInstance(Step3Provider);
				}
				stepInstance = this._step3;
				break;
			case 4: {
				const options: IStep4ReadyOptions = {
					configuredProviders: this._step3?.configuredProviders ?? [],
					importedConfig: !!(this._step2?.importSettings || this._step2?.importKeybindings || this._step2?.importExtensions),
				};
				stepInstance = new Step4Ready(options);
				break;
			}
		}

		this._currentStep = stepInstance;

		// Render into content
		clearNode(this._contentEl);

		const titleEl = document.createElement('h1');
		titleEl.className = 'forge-onboarding-title';
		titleEl.textContent = stepInstance.title;

		const subtitleEl = document.createElement('p');
		subtitleEl.className = 'forge-onboarding-subtitle';
		subtitleEl.textContent = stepInstance.subtitle;

		const bodyEl = document.createElement('div');
		stepInstance.render(bodyEl, env);

		this._contentEl.appendChild(titleEl);
		this._contentEl.appendChild(subtitleEl);
		this._contentEl.appendChild(bodyEl);
	}

	private async _onNext(): Promise<void> {
		if (!this._currentStep) {
			return;
		}

		if (!this._currentStep.validate()) {
			this._showValidationError('Please complete this step before continuing.');
			return;
		}

		this._clearValidationError();
		await this._currentStep.onNext();

		const nextStep = this._currentStepIndex + 2; // +1 for next, +1 for 1-based

		if (nextStep > TOTAL_STEPS) {
			this.onboardingService.markComplete();
			this._setState('complete');
			clearNode(this._contentEl);
			return;
		}

		this._renderStep(nextStep as 1 | 2 | 3 | 4);
	}

	private _onPrev(): void {
		const prevStep = this._currentStepIndex; // already 0-based, so index == prevStep number
		if (prevStep < 1) {
			return;
		}
		this._clearValidationError();
		this._renderStep(prevStep as 1 | 2 | 3 | 4);
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
}
