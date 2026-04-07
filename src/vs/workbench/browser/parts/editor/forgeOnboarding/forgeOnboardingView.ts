/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

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
	private readonly _shellEl: HTMLElement;
	private readonly _railEl: HTMLElement;
	private readonly _railFillEl: HTMLElement;
	private readonly _shellCounterEl: HTMLElement;
	private readonly _loadingEl: HTMLElement;
	private readonly _cardEl: HTMLElement;
	private readonly _cardHdrEl: HTMLElement;
	private readonly _eyebrowEl: HTMLElement;
	private readonly _titleEl: HTMLElement;
	private readonly _subtitleEl: HTMLElement;
	private readonly _cardBodyEl: HTMLElement;
	private readonly _cardFooterEl: HTMLElement;
	private readonly _prevBtn: HTMLButtonElement;
	private readonly _nextBtn: HTMLButtonElement;

	private _env: IEnvironmentDetectionResult | undefined;
	private _state: WizardState = 'detecting';
	private _currentStep: IOnboardingStep | undefined;

	// Keep step instances alive so selections survive navigation
	private _step2: Step2Import | undefined;
	private _step3: Step3Provider | undefined;
	private _stepCanvas: StepCanvasExplainer | undefined;
	private _stepMCP: StepMCP | undefined;
	private _step4Ready: Step4Ready | undefined;

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

		// Shell — relative container, centered
		this._shellEl = document.createElement('div');
		this._shellEl.className = 'forge-onboarding-shell';
		this._rootEl.appendChild(this._shellEl);

		// Vertical progress rail (left edge)
		this._railEl = document.createElement('div');
		this._railEl.className = 'forge-onboarding-progress-rail';

		this._railFillEl = document.createElement('div');
		this._railFillEl.className = 'forge-onboarding-progress-fill';
		this._railFillEl.style.height = '0%';
		this._railEl.appendChild(this._railFillEl);
		this._shellEl.appendChild(this._railEl);

		// Step counter — absolute top-right
		this._shellCounterEl = document.createElement('div');
		this._shellCounterEl.className = 'forge-onboarding-step-counter';
		this._shellEl.appendChild(this._shellCounterEl);

		// Loading indicator — shown only during detecting
		this._loadingEl = document.createElement('p');
		this._loadingEl.className = 'forge-onboarding-loading';
		this._loadingEl.textContent = 'Detecting your environment...';
		this._shellEl.appendChild(this._loadingEl);

		// Card
		this._cardEl = document.createElement('div');
		this._cardEl.className = 'forge-onboarding-card';
		this._shellEl.appendChild(this._cardEl);

		// Card header
		this._cardHdrEl = document.createElement('div');
		this._cardHdrEl.className = 'forge-onboarding-card-hdr';

		// Persistent Forge mark + wordmark above the eyebrow
		const markEl = document.createElement('div');
		markEl.className = 'forge-onboarding-card-hdr-mark';
		markEl.appendChild(buildForgeMarkSvg());
		const markText = document.createElement('span');
		markText.className = 'forge-onboarding-card-hdr-mark-text';
		markText.textContent = 'FORGE IDE';
		markEl.appendChild(markText);
		this._cardHdrEl.appendChild(markEl);

		this._eyebrowEl = document.createElement('div');
		this._eyebrowEl.className = 'forge-onboarding-eyebrow';
		this._cardHdrEl.appendChild(this._eyebrowEl);

		this._titleEl = document.createElement('div');
		this._titleEl.className = 'forge-onboarding-title';
		this._cardHdrEl.appendChild(this._titleEl);

		this._subtitleEl = document.createElement('p');
		this._subtitleEl.className = 'forge-onboarding-subtitle';
		this._cardHdrEl.appendChild(this._subtitleEl);

		this._cardEl.appendChild(this._cardHdrEl);

		// Card body
		this._cardBodyEl = document.createElement('div');
		this._cardBodyEl.className = 'forge-onboarding-card-body';
		this._cardEl.appendChild(this._cardBodyEl);

		// Card footer
		this._cardFooterEl = document.createElement('div');
		this._cardFooterEl.className = 'forge-onboarding-card-footer';

		this._prevBtn = document.createElement('button');
		this._prevBtn.className = 'forge-onboarding-btn-secondary';
		this._prevBtn.textContent = 'Back';

		this._nextBtn = document.createElement('button');
		this._nextBtn.className = 'forge-onboarding-btn-primary';
		this._nextBtn.textContent = 'Next';
		// Click handler is assigned per-state in _updateFooter
		// (advances on most states, launches on complete).

		this._cardFooterEl.appendChild(this._prevBtn);
		this._cardFooterEl.appendChild(this._nextBtn);
		this._cardEl.appendChild(this._cardFooterEl);

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
		this._loadingEl.style.display = isDetecting ? '' : 'none';
		this._cardEl.style.display = isDetecting ? 'none' : '';
		this._shellCounterEl.style.display = isDetecting ? 'none' : '';
		this._updateProgress(state);
	}

	private _updateProgress(state: WizardState): void {
		if (state === 'detecting') {
			this._railFillEl.style.height = '0%';
			this._eyebrowEl.textContent = '';
			this._shellCounterEl.textContent = '';
			return;
		}
		if (state === 'complete') {
			this._railFillEl.style.height = '100%';
			this._eyebrowEl.textContent = '';
			this._shellCounterEl.textContent = '';
			return;
		}
		const hasImport = this._env?.hasVSCodeConfig ?? false;
		const step = getStepNumber(state, hasImport);
		const total = getTotalSteps(hasImport);
		this._railFillEl.style.height = `${(step / total) * 100}%`;
		this._eyebrowEl.textContent = `STEP ${step} OF ${total}`;
		this._shellCounterEl.textContent = `STEP ${step} OF ${total}`;
	}

	private async _detect(): Promise<void> {
		this._env = await this.onboardingService.detectEnvironment();
		this._setState('welcome');
		this._renderCurrentStep();
	}

	private _renderCurrentStep(): void {
		const el = this._cardBodyEl;
		while (el.firstChild) { el.removeChild(el.firstChild); }
		this._clearValidationError();

		const state = this._state;

		// Ember tint on card header for complete state
		if (state === 'complete') {
			this._cardHdrEl.classList.add('ember-tint');
		} else {
			this._cardHdrEl.classList.remove('ember-tint');
		}

		switch (state) {
			case 'welcome': {
				const step = new Step1Welcome();
				this._titleEl.textContent = step.title.toUpperCase();
				this._subtitleEl.textContent = step.subtitle;
				step.render(this._cardBodyEl, this._env!);
				this._currentStep = step;
				this._updateFooter('welcome');
				break;
			}
			case 'canvas': {
				if (!this._stepCanvas) {
					this._stepCanvas = new StepCanvasExplainer();
				}
				this._titleEl.textContent = this._stepCanvas.title.toUpperCase();
				this._subtitleEl.textContent = this._stepCanvas.subtitle;
				this._stepCanvas.render(this._cardBodyEl, this._env!);
				this._currentStep = this._stepCanvas;
				this._updateFooter('canvas');
				break;
			}
			case 'import': {
				if (!this._step2) {
					this._step2 = new Step2Import();
				}
				this._titleEl.textContent = this._step2.title.toUpperCase();
				this._subtitleEl.textContent = this._step2.subtitle;
				this._step2.render(this._cardBodyEl, this._env!);
				this._currentStep = this._step2;
				this._updateFooter('import');
				break;
			}
			case 'provider': {
				if (!this._step3) {
					this._step3 = this.instantiationService.createInstance(Step3Provider);
				}
				this._titleEl.textContent = this._step3.title.toUpperCase();
				this._subtitleEl.textContent = this._step3.subtitle;
				this._step3.render(this._cardBodyEl, this._env!);
				this._currentStep = this._step3;
				this._updateFooter('provider');
				break;
			}
			case 'mcp': {
				if (!this._stepMCP) {
					this._stepMCP = new StepMCP(this.onboardingService);
				}
				this._titleEl.textContent = this._stepMCP.title.toUpperCase();
				this._subtitleEl.textContent = this._stepMCP.subtitle;
				this._stepMCP.render(this._cardBodyEl, this._env!);
				this._currentStep = this._stepMCP;
				this._updateFooter('mcp');
				break;
			}
			case 'complete': {
				const options: IStep4ReadyOptions = {
					canvasLayout: this._stepCanvas?.selectedLayout ?? 'quad',
					configuredProviders: this._step3?.configuredProviders ?? [],
					importedConfig: !!(this._step2?.importTheme || this._step2?.importKeybindings || this._step2?.importExtensions || this._step2?.importGit),
					mcpSelections: this._stepMCP?.selectedServers ?? [],
				};
				this._step4Ready = new Step4Ready(options);
				this._titleEl.textContent = this._step4Ready.title.toUpperCase();
				this._subtitleEl.textContent = this._step4Ready.subtitle;
				this._step4Ready.render(this._cardBodyEl, this._env!);
				this._currentStep = this._step4Ready;
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
		this._nextBtn.style.display = '';
		this._nextBtn.textContent = nextLabels[state] ?? 'Next';
		// On the complete step, the primary footer button launches whichever
		// action the user selected in the launch grid (defaults to New AI Session).
		// On all other steps it advances the wizard.
		this._nextBtn.onclick = state === 'complete'
			? () => void this._handleLaunch(this._step4Ready?.selectedAction ?? 'newSession')
			: () => void this._onNext();
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
		this._cardFooterEl.insertAdjacentElement('beforebegin', err);
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

const SVG_NS = 'http://www.w3.org/2000/svg';

/**
 * Builds a minimal Forge mark SVG (5-spoke hub-and-spoke from DESIGN.md section 2)
 * for the persistent card header element. Uses canonical brand hex values.
 */
function buildForgeMarkSvg(): SVGElement {
	const svg = document.createElementNS(SVG_NS, 'svg');
	svg.setAttribute('viewBox', '0 0 200 200');
	svg.setAttribute('aria-label', 'Forge IDE mark');

	const spokes: Array<[number, number, string]> = [
		[100, 20, '#ffd166'],
		[24, 75, '#ff4a12'],
		[176, 75, '#ff4a12'],
		[53, 165, '#ff7a30'],
		[147, 165, '#ff7a30'],
	];

	for (const [x, y, color] of spokes) {
		const line = document.createElementNS(SVG_NS, 'line');
		line.setAttribute('x1', '100');
		line.setAttribute('y1', '100');
		line.setAttribute('x2', String(x));
		line.setAttribute('y2', String(y));
		line.setAttribute('stroke', color);
		line.setAttribute('stroke-width', '8');
		line.setAttribute('stroke-linecap', 'round');
		svg.appendChild(line);

		const node = document.createElementNS(SVG_NS, 'circle');
		node.setAttribute('cx', String(x));
		node.setAttribute('cy', String(y));
		node.setAttribute('r', '11');
		node.setAttribute('fill', '#0c1018');
		node.setAttribute('stroke', color);
		node.setAttribute('stroke-width', '4');
		svg.appendChild(node);
	}

	const hub = document.createElementNS(SVG_NS, 'circle');
	hub.setAttribute('cx', '100');
	hub.setAttribute('cy', '100');
	hub.setAttribute('r', '24');
	hub.setAttribute('fill', '#0c1018');
	hub.setAttribute('stroke', '#1e2838');
	hub.setAttribute('stroke-width', '3');
	svg.appendChild(hub);

	return svg;
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

