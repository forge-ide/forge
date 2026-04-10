/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { IEnvironmentDetectionResult } from '../../../../../services/forge/common/forgeOnboardingService.js';
import { ISecretStorageService } from '../../../../../../platform/secrets/common/secrets.js';
import { IConfigurationService } from '../../../../../../platform/configuration/common/configuration.js';
import { IFileDialogService } from '../../../../../../platform/dialogs/common/dialogs.js';
import { IFileService } from '../../../../../../platform/files/common/files.js';
import { IOnboardingStep } from '../forgeOnboardingView.js';

interface ProviderField {
	id: string;
	label: string;
	placeholder?: string;
	envVar?: string;     // pre-fill value from this env var; shows "from environment" badge
	type?: 'text' | 'json'; // 'text' = single-line input (default), 'json' = textarea + browse
	optional?: boolean;  // if true, may be empty and step still validates
}

interface ProviderDefinition {
	id: string;
	label: string;
	description: string;
	isLocal: boolean;
	fields?: ProviderField[]; // absent → legacy single API key input
}

const PROVIDERS: ProviderDefinition[] = [
	{ id: 'anthropic', label: 'Anthropic', description: 'Claude 3.5 Sonnet, Claude 3 Haiku and more', isLocal: false },
	{ id: 'openai', label: 'OpenAI', description: 'GPT-4o, GPT-4 Turbo, o1 and more', isLocal: false },
	{
		id: 'vertex',
		label: 'Google Vertex AI',
		description: 'Gemini and Claude models via Google Cloud',
		isLocal: false,
		fields: [
			{ id: 'projectId', label: 'Project ID', envVar: 'GOOGLE_CLOUD_PROJECT', type: 'text' },
			{ id: 'location', label: 'Location', envVar: 'GOOGLE_CLOUD_LOCATION', type: 'text', placeholder: 'us-central1' },
			{ id: 'serviceAccountJson', label: 'Service Account JSON', type: 'json', optional: true },
		],
	},
	{ id: 'custom', label: 'Custom endpoint', description: 'Any OpenAI-compatible API - Azure, Together, Groq, your own', isLocal: false },
	{ id: 'local', label: 'Local - Ollama / LM Studio', description: 'No local server detected - install Ollama to use local models', isLocal: true },
];

export class Step3Provider implements IOnboardingStep {
	readonly stepId = 'provider';
	readonly title = 'CONNECT A PROVIDER';
	readonly subtitle = 'Choose how Forge talks to AI. Connect multiple providers and switch between them per pane.';

	// providerId -> api key (empty string for local/keyless providers)
	private readonly _apiKeys = new Map<string, string>();
	private readonly _checkboxes = new Map<string, HTMLInputElement>();
	private readonly _keyInputs = new Map<string, HTMLInputElement>();
	private readonly _keySections = new Map<string, HTMLElement>();
	private readonly _optionEls = new Map<string, HTMLElement>();
	private readonly _keyConfirms = new Map<string, HTMLElement>();
	// For multi-field providers: providerId -> fieldId -> value
	private readonly _fieldValues = new Map<string, Map<string, string>>();

	get configuredProviders(): string[] {
		const fromApiKeys = Array.from(this._apiKeys.keys());
		const fromFields = Array.from(this._fieldValues.keys()).filter(id => this._checkboxes.get(id)?.checked);
		return [...fromApiKeys, ...fromFields];
	}

	constructor(
		@ISecretStorageService private readonly _secretStorageService: ISecretStorageService,
		@IConfigurationService _configurationService: IConfigurationService,
		@IFileDialogService _fileDialogService: IFileDialogService,
		@IFileService _fileService: IFileService,
	) { }

	render(container: HTMLElement, env: IEnvironmentDetectionResult): void {
		// Detection banner
		const detectedProvider = Object.keys(env.detectedApiKeys)[0];
		if (detectedProvider) {
			const banner = document.createElement('div');
			banner.className = 'forge-onboarding-detect found';

			const dot = document.createElement('div');
			dot.className = 'forge-onboarding-detect-dot';
			banner.appendChild(dot);

			const providerLabel = PROVIDERS.find(p => p.id === detectedProvider)?.label ?? detectedProvider;
			banner.appendChild(document.createTextNode(`${providerLabel} API key detected`));
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

			const confirm = document.createElement('div');
			confirm.className = 'forge-onboarding-api-key-confirm';
			confirm.style.display = 'none';
			this._keyConfirms.set(provider.id, confirm);

			keyInput.addEventListener('input', () => {
				this._apiKeys.set(provider.id, keyInput.value);
				this._updateKeyConfirm(provider.id, !!detectedKey);
			});

			keySection.appendChild(keyInput);
			keySection.appendChild(confirm);
			option.appendChild(keySection);

			this._keyInputs.set(provider.id, keyInput);
			this._keySections.set(provider.id, keySection);

			// Show confirmation immediately if a key was pre-filled
			this._updateKeyConfirm(provider.id, !!detectedKey);
		}

		return option;
	}

	private _updateKeyConfirm(providerId: string, fromEnvironment: boolean): void {
		const confirm = this._keyConfirms.get(providerId);
		const keyInput = this._keyInputs.get(providerId);
		if (!confirm || !keyInput) {
			return;
		}
		if (keyInput.value.length > 0) {
			const masked = this._maskKey(keyInput.value);
			confirm.textContent = fromEnvironment
				? `Key found in environment - ${masked}`
				: `Key set - ${masked}`;
			confirm.style.display = '';
		} else {
			confirm.style.display = 'none';
		}
	}

	private _maskKey(key: string): string {
		if (key.length <= 8) {
			return '*'.repeat(key.length);
		}
		return `${key.slice(0, 6)}${'*'.repeat(Math.max(8, key.length - 10))}${key.slice(-4)}`;
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
