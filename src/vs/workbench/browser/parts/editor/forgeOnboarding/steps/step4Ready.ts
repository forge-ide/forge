/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { IEnvironmentDetectionResult } from '../../../../../services/forge/common/forgeOnboardingService.js';
import { IOnboardingStep } from '../forgeOnboardingView.js';
import { CanvasLayout, canvasLayoutLabel } from './stepCanvasExplainer.js';

export type LaunchAction = 'openFolder' | 'newSession';

export interface IStep4ReadyOptions {
	canvasLayout: CanvasLayout;
	configuredProviders: string[];
	importedConfig: boolean;
	mcpSelections: string[];
}

const SVG_NS = 'http://www.w3.org/2000/svg';

const FOLDER_ICON_PATHS = ['M3 7a1 1 0 0 1 1-1h5l2 2h9a1 1 0 0 1 1 1v10a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V7z'];
const HEX_ICON_PATHS = ['M12 2 21 7v10l-9 5-9-5V7l9-5z', 'M12 8v8M8 10v4M16 10v4'];

function createSvgIcon(paths: string[]): SVGElement {
	const svg = document.createElementNS(SVG_NS, 'svg');
	svg.setAttribute('viewBox', '0 0 24 24');
	svg.setAttribute('stroke-width', '1.5');
	svg.setAttribute('stroke-linecap', 'round');
	svg.setAttribute('stroke-linejoin', 'round');
	for (const d of paths) {
		const path = document.createElementNS(SVG_NS, 'path');
		path.setAttribute('d', d);
		svg.appendChild(path);
	}
	return svg;
}

export class Step4Ready implements IOnboardingStep {
	readonly stepId = 'ready';
	readonly title = 'READY TO FORGE';
	readonly subtitle = 'Your workspace is configured. Open a folder or start a new AI session to begin.';

	private _selectedAction: LaunchAction = 'newSession';

	get selectedAction(): LaunchAction {
		return this._selectedAction;
	}

	constructor(private readonly options: IStep4ReadyOptions) { }

	render(container: HTMLElement, _env: IEnvironmentDetectionResult): void {
		// Checkmark summary
		const summary = document.createElement('div');
		summary.className = 'forge-onboarding-summary';

		summary.appendChild(this._summaryRow(`Canvas layout set to ${canvasLayoutLabel(this.options.canvasLayout)}`));

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

		// Launch grid — cards are selection toggles. The footer "Enter Forge" button
		// fires the chosen action; clicking a card here just selects it.
		const grid = document.createElement('div');
		grid.className = 'forge-onboarding-launch-grid';

		const cards = new Map<LaunchAction, HTMLElement>();

		const openFolderBtn = this._launchBtn(createSvgIcon(FOLDER_ICON_PATHS), 'Open Folder', 'Start with an existing project');
		cards.set('openFolder', openFolderBtn);
		grid.appendChild(openFolderBtn);

		const newSessionBtn = this._launchBtn(createSvgIcon(HEX_ICON_PATHS), 'New AI Session', 'Start with a blank canvas');
		cards.set('newSession', newSessionBtn);
		grid.appendChild(newSessionBtn);

		for (const [action, card] of cards) {
			card.classList.toggle('selected', action === this._selectedAction);
			card.addEventListener('click', () => {
				this._selectedAction = action;
				for (const [otherAction, otherCard] of cards) {
					otherCard.classList.toggle('selected', otherAction === action);
				}
			});
		}

		container.appendChild(grid);
	}

	validate(): boolean {
		return true;
	}

	async onNext(): Promise<void> {
		// main nav reads selectedAction and dispatches via _handleLaunch
	}

	private _summaryRow(text: string): HTMLElement {
		const row = document.createElement('div');
		row.className = 'forge-onboarding-summary-row';

		const check = document.createElement('span');
		check.className = 'forge-onboarding-summary-check';
		// Glyph is rendered via CSS ::before to keep source ASCII-only
		row.appendChild(check);

		const label = document.createElement('span');
		label.textContent = text;
		row.appendChild(label);

		return row;
	}

	private _launchBtn(icon: SVGElement, label: string, subtitle: string): HTMLElement {
		const btn = document.createElement('div');
		btn.className = 'forge-onboarding-launch-btn';

		const iconEl = document.createElement('span');
		iconEl.className = 'forge-onboarding-launch-icon';
		iconEl.appendChild(icon);
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
