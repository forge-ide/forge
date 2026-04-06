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
	readonly title = 'Configure AI Providers';
	readonly subtitle = 'Enable one or more providers to power Forge';

	// providerId -> api key (empty string for local/keyless providers)
	private readonly _apiKeys = new Map<string, string>();
	private readonly _checkboxes = new Map<string, HTMLInputElement>();
	private readonly _keyInputs = new Map<string, HTMLInputElement>();
	private readonly _keySections = new Map<string, HTMLElement>();
	private readonly _optionEls = new Map<string, HTMLElement>();

	get configuredProviders(): string[] {
		return Array.from(this._apiKeys.keys());
	}

	constructor(
		@ISecretStorageService private readonly secretStorageService: ISecretStorageService,
	) { }

	render(container: HTMLElement, env: IEnvironmentDetectionResult): void {
		const list = document.createElement('div');
		list.className = 'forge-onboarding-provider-list';

		for (const provider of PROVIDERS) {
			list.appendChild(this._buildProviderOption(provider, env));
		}

		container.appendChild(list);

		// Auto-check providers with detected keys or running local servers
		for (const provider of PROVIDERS) {
			if (!provider.isLocal && env.detectedApiKeys[provider.id]) {
				this._setChecked(provider.id, true, env.detectedApiKeys[provider.id]);
			} else if (provider.isLocal && (env.ollamaRunning || env.lmStudioRunning)) {
				this._setChecked(provider.id, true, '');
			}
		}
	}

	validate(): boolean {
		for (const [providerId, key] of this._apiKeys) {
			const def = PROVIDERS.find(p => p.id === providerId);
			if (def?.isLocal || key.length > 0) {
				return true;
			}
		}
		return false;
	}

	async onNext(): Promise<void> {
		for (const [providerId, key] of this._apiKeys) {
			if (key) {
				await this.secretStorageService.set(`forge.apikey.${providerId}`, key);
			}
		}
	}

	private _buildProviderOption(provider: ProviderDefinition, env: IEnvironmentDetectionResult): HTMLElement {
		const wrapper = document.createElement('div');
		wrapper.className = 'forge-onboarding-provider-option';
		this._optionEls.set(provider.id, wrapper);

		// Header row
		const header = document.createElement('div');
		header.className = 'forge-onboarding-provider-header';

		const checkbox = document.createElement('input');
		checkbox.type = 'checkbox';
		checkbox.className = 'forge-onboarding-provider-checkbox';
		checkbox.value = provider.id;
		this._checkboxes.set(provider.id, checkbox);

		const labelEl = document.createElement('span');
		labelEl.className = 'forge-onboarding-provider-label';
		labelEl.textContent = provider.label;

		header.appendChild(checkbox);
		header.appendChild(labelEl);

		// Detected badge
		if (!provider.isLocal && env.detectedApiKeys[provider.id]) {
			const badge = document.createElement('span');
			badge.className = 'forge-onboarding-provider-detected';
			badge.textContent = 'Key detected';
			header.appendChild(badge);
		} else if (provider.isLocal && (env.ollamaRunning || env.lmStudioRunning)) {
			const parts: string[] = [];
			if (env.ollamaRunning) { parts.push('Ollama'); }
			if (env.lmStudioRunning) { parts.push('LM Studio'); }
			const badge = document.createElement('span');
			badge.className = 'forge-onboarding-provider-detected';
			badge.textContent = `${parts.join(', ')} running`;
			header.appendChild(badge);
		}

		wrapper.appendChild(header);

		// API key section (cloud providers only)
		if (!provider.isLocal) {
			const keySection = document.createElement('div');
			keySection.className = 'forge-onboarding-api-key-section';
			keySection.style.display = 'none';

			const keyInput = document.createElement('input');
			keyInput.type = 'password';
			keyInput.className = 'forge-onboarding-api-key-input';
			keyInput.placeholder = 'Paste your API key\u2026';
			keyInput.addEventListener('input', () => {
				if (this._apiKeys.has(provider.id)) {
					this._apiKeys.set(provider.id, keyInput.value);
				}
			});

			keySection.appendChild(keyInput);
			wrapper.appendChild(keySection);

			this._keyInputs.set(provider.id, keyInput);
			this._keySections.set(provider.id, keySection);
		}

		// Clicking the header (outside the checkbox) toggles the checkbox
		header.addEventListener('click', (e) => {
			if (e.target !== checkbox) {
				checkbox.checked = !checkbox.checked;
				this._setChecked(provider.id, checkbox.checked, env.detectedApiKeys[provider.id] ?? '');
			}
		});

		checkbox.addEventListener('change', () => {
			this._setChecked(provider.id, checkbox.checked, env.detectedApiKeys[provider.id] ?? '');
		});

		return wrapper;
	}

	private _setChecked(providerId: string, checked: boolean, prefillKey: string): void {
		const checkbox = this._checkboxes.get(providerId);
		if (checkbox) {
			checkbox.checked = checked;
		}

		const optionEl = this._optionEls.get(providerId);
		optionEl?.classList.toggle('checked', checked);

		const keySection = this._keySections.get(providerId);
		const keyInput = this._keyInputs.get(providerId);
		const def = PROVIDERS.find(p => p.id === providerId);

		if (checked) {
			if (def?.isLocal) {
				this._apiKeys.set(providerId, '');
			} else {
				if (keyInput && !keyInput.value && prefillKey) {
					keyInput.value = prefillKey;
				}
				this._apiKeys.set(providerId, keyInput?.value ?? prefillKey);
				if (keySection) { keySection.style.display = 'block'; }
			}
		} else {
			this._apiKeys.delete(providerId);
			if (keySection) { keySection.style.display = 'none'; }
		}
	}
}
