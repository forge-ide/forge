/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { IEnvironmentDetectionResult } from '../../../../../services/forge/common/forgeOnboardingService.js';
import { IOnboardingStep } from '../forgeOnboardingView.js';

export interface IStep4ReadyOptions {
	selectedProvider: string | undefined;
	importedConfig: boolean;
}

export class Step4Ready implements IOnboardingStep {
	readonly stepId = 'ready';
	readonly title = 'Setup complete';
	readonly subtitle = 'Forge is configured and ready to use.';

	constructor(private readonly options: IStep4ReadyOptions) { }

	render(container: HTMLElement, _env: IEnvironmentDetectionResult): void {
		const summary = document.createElement('div');
		summary.className = 'forge-onboarding-summary';

		if (this.options.selectedProvider) {
			summary.appendChild(this._summaryRow(`AI provider configured: ${this._providerLabel(this.options.selectedProvider)}`));
		}

		if (this.options.importedConfig) {
			summary.appendChild(this._summaryRow('VS Code config imported'));
		}

		summary.appendChild(this._summaryRow('MCP tool support enabled'));

		container.appendChild(summary);

		const links = document.createElement('div');
		links.className = 'forge-onboarding-links';
		links.appendChild(this._link('Documentation', '#'));
		links.appendChild(this._link('GitHub', '#'));
		links.appendChild(this._link('Discord', '#'));
		container.appendChild(links);
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
		check.textContent = '✓';

		row.appendChild(check);
		row.appendChild(document.createTextNode(text));
		return row;
	}

	private _link(label: string, href: string): HTMLElement {
		const a = document.createElement('a');
		a.className = 'forge-onboarding-link';
		a.href = href;
		a.textContent = label;
		a.target = '_blank';
		a.rel = 'noopener noreferrer';
		return a;
	}

	private _providerLabel(id: string): string {
		const labels: Record<string, string> = {
			anthropic: 'Anthropic',
			openai: 'OpenAI',
			gemini: 'Gemini',
			local: 'Local (Ollama / LM Studio)',
		};
		return labels[id] ?? id;
	}
}
