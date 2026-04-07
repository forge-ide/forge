/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { IEnvironmentDetectionResult } from '../../../../../services/forge/common/forgeOnboardingService.js';
import { IOnboardingStep } from '../forgeOnboardingView.js';

export interface IStep4ReadyOptions {
	configuredProviders: string[];
	importedConfig: boolean;
	mcpSelections: string[];
	onLaunch: (action: 'openFolder' | 'newSession') => Promise<void>;
}

export class Step4Ready implements IOnboardingStep {
	readonly stepId = 'ready';
	readonly title = 'Setup complete';
	readonly subtitle = 'Forge is configured and ready to use.';

	constructor(private readonly options: IStep4ReadyOptions) { }

	render(container: HTMLElement, _env: IEnvironmentDetectionResult): void {
		// Ember-tinted header zone
		const header = document.createElement('div');
		header.className = 'forge-onboarding-ready-header';

		const titleEl = document.createElement('div');
		titleEl.className = 'forge-onboarding-title';
		titleEl.textContent = 'READY TO FORGE';
		header.appendChild(titleEl);

		const subtitleEl = document.createElement('p');
		subtitleEl.className = 'forge-onboarding-subtitle';
		subtitleEl.textContent = 'Your workspace is configured. Open a folder or start a new AI session to begin.';
		header.appendChild(subtitleEl);

		container.appendChild(header);

		// Checkmark summary
		const summary = document.createElement('div');
		summary.className = 'forge-onboarding-summary';

		summary.appendChild(this._summaryRow('Canvas layout set to Quad'));

		if (this.options.importedConfig) {
			summary.appendChild(this._summaryRow('VS Code config imported - keybindings, theme, extensions'));
		}

		for (const providerId of this.options.configuredProviders) {
			summary.appendChild(this._summaryRow(`${this._providerLabel(providerId)} connected`));
		}

		if (this.options.mcpSelections.length > 0) {
			const mcpLabels = this.options.mcpSelections.map(id => this._mcpLabel(id)).join(' - ');
			summary.appendChild(this._summaryRow(`${mcpLabels} enabled`));
		}

		container.appendChild(summary);

		// Launch grid
		const grid = document.createElement('div');
		grid.className = 'forge-onboarding-launch-grid';

		const openFolderBtn = this._launchBtn('[folder]', 'Open Folder', 'Start with an existing project', false);
		openFolderBtn.addEventListener('click', () => void this.options.onLaunch('openFolder'));
		grid.appendChild(openFolderBtn);

		const newSessionBtn = this._launchBtn('[ai]', 'New AI Session', 'Start with a blank canvas', true);
		newSessionBtn.addEventListener('click', () => void this.options.onLaunch('newSession'));
		grid.appendChild(newSessionBtn);

		container.appendChild(grid);
	}

	validate(): boolean {
		return true;
	}

	async onNext(): Promise<void> {
		// main nav calls onboardingService.markComplete() and closes the editor
	}

	private _summaryRow(text: string): HTMLElement {
		const row = document.createElement('div');
		row.className = 'forge-onboarding-summary-row';

		const check = document.createElement('span');
		check.className = 'forge-onboarding-summary-check';
		check.textContent = '+';  // ASCII for checkmark (hygiene rejects unicode)
		row.appendChild(check);

		const label = document.createElement('span');
		label.textContent = text;
		row.appendChild(label);

		return row;
	}

	private _launchBtn(icon: string, label: string, subtitle: string, highlighted: boolean): HTMLElement {
		const btn = document.createElement('div');
		btn.className = 'forge-onboarding-launch-btn' + (highlighted ? ' highlighted' : '');

		const iconEl = document.createElement('span');
		iconEl.className = 'forge-onboarding-launch-icon';
		iconEl.textContent = icon;
		btn.appendChild(iconEl);

		const labelEl = document.createElement('div');
		labelEl.textContent = label;
		btn.appendChild(labelEl);

		const subEl = document.createElement('div');
		subEl.className = 'forge-onboarding-launch-subtitle';
		subEl.textContent = subtitle;
		btn.appendChild(subEl);

		return btn;
	}

	private _mcpLabel(id: string): string {
		const labels: Record<string, string> = {
			filesystem: 'Filesystem MCP',
			github: 'GitHub MCP',
			browser: 'Browser MCP',
			postgres: 'Postgres MCP',
		};
		return labels[id] ?? id;
	}

	private _providerLabel(id: string): string {
		const labels: Record<string, string> = {
			anthropic: 'Anthropic',
			openai: 'OpenAI',
			custom: 'Custom endpoint',
			local: 'Local (Ollama / LM Studio)',
		};
		return labels[id] ?? id;
	}
}
