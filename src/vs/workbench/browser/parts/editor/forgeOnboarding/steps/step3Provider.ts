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
	description: string;
	isLocal: boolean;
}

const PROVIDERS: ProviderDefinition[] = [
	{ id: 'anthropic', label: 'Anthropic', description: 'Claude 3.5 Sonnet, Claude 3 Haiku and more', isLocal: false },
	{ id: 'openai', label: 'OpenAI', description: 'GPT-4o, GPT-4 Turbo, o1 and more', isLocal: false },
	{ id: 'custom', label: 'Custom endpoint', description: 'Any OpenAI-compatible API - Azure, Together, Groq, your own', isLocal: false },
	{ id: 'local', label: 'Local - Ollama / LM Studio', description: 'No local server detected - install Ollama to use local models', isLocal: true },
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
		@ISecretStorageService private readonly _secretStorageService: ISecretStorageService,
	) { }

	render(container: HTMLElement, env: IEnvironmentDetectionResult): void {
		// Detection banner
		const detectedProvider = Object.keys(env.detectedApiKeys)[0];
		if (detectedProvider) {
			const banner = document.createElement('div');
			banner.className = 'forge-onboarding-detection-banner';
			const providerLabel = PROVIDERS.find(p => p.id === detectedProvider)?.label ?? detectedProvider;
			banner.textContent = `${providerLabel} API key detected`;
			container.appendChild(banner);
		}

		const list = document.createElement('div');
		list.className = 'forge-onboarding-provider-list';

		for (const provider of PROVIDERS) {
			list.appendChild(this._buildProviderOption(provider, env));
		}

		container.appendChild(list);
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
		for (const [providerId, key] of this._apiKeys.entries()) {
			if (key && this._checkboxes.get(providerId)?.checked) {
				await this._secretStorageService.set(`forge.provider.apiKey.${providerId}`, key);
			}
		}
	}

	private _buildProviderOption(provider: ProviderDefinition, env: IEnvironmentDetectionResult): HTMLElement {
		const isDetectedLocal = provider.isLocal && (env.ollamaRunning || env.lmStudioRunning);
		const isDimmed = provider.isLocal && !env.ollamaRunning && !env.lmStudioRunning;

		const option = document.createElement('div');
		option.className = 'forge-onboarding-provider-option' + (isDimmed ? ' dim' : '');

		const header = document.createElement('div');
		header.className = 'forge-onboarding-provider-header';

		const checkbox = document.createElement('input');
		checkbox.type = 'checkbox';
		checkbox.className = 'forge-onboarding-provider-checkbox';
		checkbox.id = `provider-${provider.id}`;

		const hex = document.createElement('div');
		hex.className = 'forge-onboarding-provider-hex';

		const labelWrap = document.createElement('div');
		labelWrap.className = 'forge-onboarding-provider-label';

		const labelEl = document.createElement('span');
		labelEl.textContent = provider.label;
		labelWrap.appendChild(labelEl);

		const descEl = document.createElement('div');
		descEl.className = 'forge-onboarding-provider-detected';
		descEl.textContent = provider.description;
		labelWrap.appendChild(descEl);

		const typeBadge = document.createElement('span');
		typeBadge.className = 'forge-onboarding-provider-type-badge';
		typeBadge.textContent = provider.isLocal ? 'local' : 'cloud';

		header.appendChild(checkbox);
		header.appendChild(hex);
		header.appendChild(labelWrap);
		header.appendChild(typeBadge);
		option.appendChild(header);

		// Auto-detect pre-selection
		const detectedKey = env.detectedApiKeys[provider.id] ?? '';
		if (detectedKey || isDetectedLocal) {
			this._setChecked(provider.id, true, detectedKey);
			checkbox.checked = true;
			option.classList.add('checked');
		}

		this._checkboxes.set(provider.id, checkbox);
		this._optionEls.set(provider.id, option);

		checkbox.addEventListener('change', () => {
			this._setChecked(provider.id, checkbox.checked, this._apiKeys.get(provider.id) ?? '');
			option.classList.toggle('checked', checkbox.checked);
		});

		// API key section (for non-local providers)
		if (!provider.isLocal) {
			const keySection = document.createElement('div');
			keySection.className = 'forge-onboarding-api-key-section';
			keySection.style.display = checkbox.checked ? '' : 'none';

			const keyInput = document.createElement('input');
			keyInput.type = 'password';
			keyInput.className = 'forge-onboarding-api-key-input';
			keyInput.placeholder = `Enter ${provider.label} API key`;
			if (detectedKey) {
				keyInput.value = detectedKey;
			}

			keyInput.addEventListener('input', () => {
				this._apiKeys.set(provider.id, keyInput.value);
			});

			keySection.appendChild(keyInput);
			option.appendChild(keySection);

			this._keyInputs.set(provider.id, keyInput);
			this._keySections.set(provider.id, keySection);
		}

		return option;
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
