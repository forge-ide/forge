/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { IEnvironmentDetectionResult } from '../../../../../services/forge/common/forgeOnboardingService.js';
import { ISecretStorageService } from '../../../../../../platform/secrets/common/secrets.js';
import { IOnboardingStep } from '../forgeOnboardingView.js';

interface ProviderDefinition {
	id: string;
	label: string;
	isLocal: boolean;
}

const PROVIDERS: ProviderDefinition[] = [
	{ id: 'anthropic', label: 'Anthropic', isLocal: false },
	{ id: 'openai', label: 'OpenAI', isLocal: false },
	{ id: 'gemini', label: 'Gemini', isLocal: false },
	{ id: 'local', label: 'Local (Ollama / LM Studio)', isLocal: true },
];

export class Step3Provider implements IOnboardingStep {
	readonly stepId = 'provider';
	readonly title = 'Configure AI Provider';
	readonly subtitle = 'Set up at least one AI provider to get started';

	selectedProvider: string | undefined;
	apiKey = '';

	private _env: IEnvironmentDetectionResult | undefined;
	private _apiKeyInput: HTMLInputElement | undefined;
	private _apiKeySection: HTMLElement | undefined;
	private readonly _radioInputs = new Map<string, HTMLInputElement>();

	constructor(
		@ISecretStorageService private readonly secretStorageService: ISecretStorageService,
	) { }

	render(container: HTMLElement, env: IEnvironmentDetectionResult): void {
		this._env = env;

		const list = document.createElement('div');
		list.className = 'forge-onboarding-provider-list';

		for (const provider of PROVIDERS) {
			list.appendChild(this._buildProviderOption(provider, env));
		}

		container.appendChild(list);

		const apiKeySection = document.createElement('div');
		apiKeySection.className = 'forge-onboarding-api-key-section';
		apiKeySection.style.display = 'none';

		const label = document.createElement('span');
		label.className = 'forge-onboarding-api-key-label';
		label.textContent = 'API Key';

		const input = document.createElement('input');
		input.type = 'password';
		input.className = 'forge-onboarding-api-key-input';
		input.placeholder = 'Paste your API key…';
		input.addEventListener('input', () => { this.apiKey = input.value; });

		apiKeySection.appendChild(label);
		apiKeySection.appendChild(input);
		container.appendChild(apiKeySection);

		this._apiKeyInput = input;
		this._apiKeySection = apiKeySection;

		// Pre-select if a key was detected
		const detectedProvider = this._firstDetectedCloudProvider(env);
		if (detectedProvider) {
			this._selectProvider(detectedProvider, list, apiKeySection);
		} else if (env.ollamaRunning || env.lmStudioRunning) {
			this._selectProvider('local', list, apiKeySection);
		}
	}

	validate(): boolean {
		return this.selectedProvider !== undefined;
	}

	async onNext(): Promise<void> {
		if (this.selectedProvider && this.apiKey) {
			await this.secretStorageService.set(`forge.apikey.${this.selectedProvider}`, this.apiKey);
		}
	}

	private _buildProviderOption(provider: ProviderDefinition, env: IEnvironmentDetectionResult): HTMLElement {
		const row = document.createElement('div');
		row.className = 'forge-onboarding-provider-option';
		row.dataset['providerId'] = provider.id;

		const radio = document.createElement('input');
		radio.type = 'radio';
		radio.name = 'forge-onboarding-provider';
		radio.className = 'forge-onboarding-provider-radio';
		radio.value = provider.id;
		this._radioInputs.set(provider.id, radio);

		const labelEl = document.createElement('span');
		labelEl.className = 'forge-onboarding-provider-label';
		labelEl.textContent = provider.label;

		row.appendChild(radio);
		row.appendChild(labelEl);

		if (!provider.isLocal && env.detectedApiKeys[provider.id]) {
			const detected = document.createElement('span');
			detected.className = 'forge-onboarding-provider-detected';
			detected.textContent = 'Key detected from environment';
			row.appendChild(detected);
		}

		if (provider.id === 'local' && (env.ollamaRunning || env.lmStudioRunning)) {
			const detected = document.createElement('span');
			detected.className = 'forge-onboarding-provider-detected';
			const parts: string[] = [];
			if (env.ollamaRunning) { parts.push('Ollama'); }
			if (env.lmStudioRunning) { parts.push('LM Studio'); }
			detected.textContent = `${parts.join(', ')} running`;
			row.appendChild(detected);
		}

		row.addEventListener('click', () => {
			const list = row.parentElement;
			const apiKeySection = this._apiKeySection;
			if (list && apiKeySection) {
				this._selectProvider(provider.id, list, apiKeySection);
			}
		});

		return row;
	}

	private _selectProvider(providerId: string, list: HTMLElement, apiKeySection: HTMLElement): void {
		this.selectedProvider = providerId;

		for (const row of Array.from(list.children) as HTMLElement[]) {
			const isSelected = row.dataset['providerId'] === providerId;
			row.classList.toggle('selected', isSelected);
			const radio = this._radioInputs.get(row.dataset['providerId'] ?? '');
			if (radio) {
				radio.checked = isSelected;
			}
		}

		const selectedDef = PROVIDERS.find(p => p.id === providerId);
		const showApiKey = selectedDef !== undefined && !selectedDef.isLocal;
		apiKeySection.style.display = showApiKey ? 'block' : 'none';

		// Pre-fill key if detected
		if (showApiKey && this._env && this._apiKeyInput) {
			const detected = this._env.detectedApiKeys[providerId];
			if (detected) {
				this._apiKeyInput.value = detected;
				this.apiKey = detected;
			}
		}
	}

	private _firstDetectedCloudProvider(env: IEnvironmentDetectionResult): string | undefined {
		for (const provider of PROVIDERS) {
			if (!provider.isLocal && env.detectedApiKeys[provider.id]) {
				return provider.id;
			}
		}
		return undefined;
	}
}
