/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { IEnvironmentDetectionResult } from '../../../../../services/forge/common/forgeOnboardingService.js';
import { ISecretStorageService } from '../../../../../../platform/secrets/common/secrets.js';
import { IConfigurationService, ConfigurationTarget } from '../../../../../../platform/configuration/common/configuration.js';
import { IFileDialogService } from '../../../../../../platform/dialogs/common/dialogs.js';
import { IFileService } from '../../../../../../platform/files/common/files.js';
import { ILogService } from '../../../../../../platform/log/common/log.js';
import { IOnboardingStep } from '../forgeOnboardingView.js';
import { ForgeProviderConfig } from '../../../../../services/forge/common/forgeConfigTypes.js';

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
		@IConfigurationService private readonly _configurationService: IConfigurationService,
		@IFileDialogService private readonly _fileDialogService: IFileDialogService,
		@IFileService private readonly _fileService: IFileService,
		@ILogService private readonly _logService: ILogService,
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
		// Field-based providers (e.g. Vertex)
		for (const provider of PROVIDERS) {
			if (!provider.fields) { continue; }
			const checkbox = this._checkboxes.get(provider.id);
			if (!checkbox?.checked) { continue; }
			const values = this._fieldValues.get(provider.id);
			const allRequiredSet = provider.fields
				.filter(f => !f.optional)
				.every(f => (values?.get(f.id) ?? '').length > 0);
			if (allRequiredSet) { return true; }
		}

		// Legacy single-key providers
		for (const [providerId, key] of this._apiKeys) {
			const def = PROVIDERS.find(p => p.id === providerId);
			if (def?.isLocal || key.length > 0) {
				return true;
			}
		}
		return false;
	}

	async onNext(): Promise<void> {
		// Legacy single-key providers
		for (const [providerId, key] of this._apiKeys.entries()) {
			if (key && this._checkboxes.get(providerId)?.checked) {
				await this._secretStorageService.set(`forge.provider.apiKey.${providerId}`, key);
			}
		}

		// Field-based providers
		for (const provider of PROVIDERS) {
			if (!provider.fields) { continue; }
			const checkbox = this._checkboxes.get(provider.id);
			if (!checkbox?.checked) { continue; }

			const values = this._fieldValues.get(provider.id) ?? new Map<string, string>();

			if (provider.id === 'vertex') {
				const projectId = values.get('projectId') ?? '';
				const location = values.get('location') ?? '';
				const serviceAccountJson = values.get('serviceAccountJson') ?? '';

				const currentProviders = this._configurationService.getValue<ForgeProviderConfig[]>('forge.providers') ?? [];
				const withoutVertex = currentProviders.filter(p => p.name !== 'vertex');
				const vertexEntry: ForgeProviderConfig = { name: 'vertex', projectId, location, models: [] };
				await this._configurationService.updateValue(
					'forge.providers',
					[...withoutVertex, vertexEntry],
					ConfigurationTarget.USER,
				);

				await this._secretStorageService.set('forge.provider.apiKey.vertex', serviceAccountJson);
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

		// Form section: multi-field (vertex) or single API key
		if (!provider.isLocal) {
			if (provider.fields) {
				// Multi-field provider (e.g. Vertex)
				const fieldsSection = document.createElement('div');
				fieldsSection.className = 'forge-onboarding-api-key-section';
				fieldsSection.style.display = checkbox.checked ? '' : 'none';
				fieldsSection.appendChild(this._buildFieldsForm(provider, env));
				option.appendChild(fieldsSection);
				this._keySections.set(provider.id, fieldsSection);

				// Auto-check when env vars are fully set
				const hasFullVertexEnv = provider.id === 'vertex' && !!env.vertexEnv.projectId && !!env.vertexEnv.location;
				if (hasFullVertexEnv && !checkbox.checked) {
					this._setChecked(provider.id, true, '');
					option.classList.add('checked');
				}
			} else {
				// Legacy single-key provider
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

				this._updateKeyConfirm(provider.id, !!detectedKey);
			}
		}

		return option;
	}

	private _buildFieldsForm(provider: ProviderDefinition, env: IEnvironmentDetectionResult): HTMLElement {
		const form = document.createElement('div');
		form.className = 'forge-onboarding-fields-form';

		if (!this._fieldValues.has(provider.id)) {
			this._fieldValues.set(provider.id, new Map());
		}
		const values = this._fieldValues.get(provider.id)!;

		for (const field of provider.fields ?? []) {
			const fieldRow = document.createElement('div');
			fieldRow.className = 'forge-onboarding-field-row';

			// Label + optional env badge
			const labelRow = document.createElement('div');
			labelRow.className = 'forge-onboarding-field-label-row';

			const labelEl = document.createElement('label');
			labelEl.textContent = field.label;
			if (field.optional) {
				const optionalBadge = document.createElement('span');
				optionalBadge.className = 'forge-onboarding-field-optional';
				optionalBadge.textContent = 'optional';
				labelEl.appendChild(optionalBadge);
			}
			labelRow.appendChild(labelEl);

			fieldRow.appendChild(labelRow);

			if (field.type === 'json') {
				const browseLink = document.createElement('button');
				browseLink.type = 'button';
				browseLink.className = 'forge-onboarding-json-browse';
				browseLink.textContent = 'Browse...';
				labelRow.appendChild(browseLink);

				const textarea = document.createElement('textarea');
				textarea.className = 'forge-onboarding-provider-json-input';
				textarea.placeholder = 'Paste service account JSON or use Browse';
				textarea.rows = 4;
				textarea.addEventListener('input', () => {
					values.set(field.id, textarea.value);
				});

				browseLink.addEventListener('click', () => {
					void this._browseForJson(json => {
						textarea.value = json;
						values.set(field.id, json);
					});
				});

				fieldRow.appendChild(textarea);
			} else {
				// Determine pre-fill value from vertexEnv
				let prefillValue = '';
				if (field.envVar === 'GOOGLE_CLOUD_PROJECT' && env.vertexEnv.projectId) {
					prefillValue = env.vertexEnv.projectId;
				} else if (field.envVar === 'GOOGLE_CLOUD_LOCATION' && env.vertexEnv.location) {
					prefillValue = env.vertexEnv.location;
				}

				const input = document.createElement('input');
				input.type = 'text';
				input.className = 'forge-onboarding-provider-field-input';
				if (field.placeholder) { input.placeholder = field.placeholder; }
				if (prefillValue) {
					input.value = prefillValue;
					values.set(field.id, prefillValue);

					const envBadge = document.createElement('span');
					envBadge.className = 'forge-onboarding-field-env-badge';
					envBadge.textContent = 'from environment';
					labelRow.appendChild(envBadge);
				}
				input.addEventListener('input', () => {
					values.set(field.id, input.value);
				});
				fieldRow.appendChild(input);
			}

			form.appendChild(fieldRow);
		}

		return form;
	}

	private async _browseForJson(onSelect: (json: string) => void): Promise<void> {
		try {
			const uris = await this._fileDialogService.showOpenDialog({
				title: 'Select Service Account JSON',
				canSelectFiles: true,
				canSelectFolders: false,
				canSelectMany: false,
				filters: [{ name: 'JSON Files', extensions: ['json'] }],
			});
			if (!uris || uris.length === 0) { return; }
			const content = await this._fileService.readFile(uris[0]);
			onSelect(content.value.toString());
		} catch (err) {
			this._logService.error('[Step3Provider] Failed to read service account JSON file', err);
		}
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
			} else if (def?.fields) {
				// Multi-field provider: initialize values map if not already set
				if (!this._fieldValues.has(providerId)) {
					this._fieldValues.set(providerId, new Map());
				}
				if (keySection) { keySection.style.display = 'block'; }
			} else {
				if (keyInput && !keyInput.value && prefillKey) {
					keyInput.value = prefillKey;
				}
				this._apiKeys.set(providerId, keyInput?.value ?? prefillKey);
				if (keySection) { keySection.style.display = 'block'; }
			}
		} else {
			this._apiKeys.delete(providerId);
			// Do NOT delete _fieldValues here — values persist across uncheck/recheck so pre-filled
			// env values aren't lost. onNext() and validate() both gate on checkbox.checked.
			if (keySection) { keySection.style.display = 'none'; }
		}
	}
}
