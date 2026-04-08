/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { IEnvironmentDetectionResult } from '../../../../../services/forge/common/forgeOnboardingService.js';
import { IOnboardingStep } from '../forgeOnboardingView.js';

export type CanvasLayout = 'focus' | 'split' | 'quad' | 'code-ai';

interface PaneSpec {
	dotClass: 'green' | 'steel' | 'amber';
	label: string;
	cursor?: boolean;
}

interface LayoutDefinition {
	id: CanvasLayout;
	label: string;
	panes: PaneSpec[];
}

const LAYOUTS: LayoutDefinition[] = [
	{
		id: 'focus',
		label: 'Focus',
		panes: [
			{ dotClass: 'green', label: 'AI chat', cursor: true },
		],
	},
	{
		id: 'split',
		label: 'Split',
		panes: [
			{ dotClass: 'green', label: 'AI chat', cursor: true },
			{ dotClass: 'amber', label: 'Code' },
		],
	},
	{
		id: 'quad',
		label: 'Quad',
		panes: [
			{ dotClass: 'green', label: 'AI chat', cursor: true },
			{ dotClass: 'steel', label: 'AI chat' },
			{ dotClass: 'amber', label: 'Code' },
			{ dotClass: 'steel', label: 'AI chat' },
		],
	},
	{
		id: 'code-ai',
		label: 'Code + AI',
		panes: [
			{ dotClass: 'amber', label: 'Code' },
			{ dotClass: 'green', label: 'AI chat', cursor: true },
		],
	},
];

const LAYOUT_LABELS: Record<CanvasLayout, string> = {
	'focus': 'Focus',
	'split': 'Split',
	'quad': 'Quad',
	'code-ai': 'Code + AI',
};

export function canvasLayoutLabel(layout: CanvasLayout): string {
	return LAYOUT_LABELS[layout];
}

export class StepCanvasExplainer implements IOnboardingStep {
	readonly stepId = 'canvas';
	readonly title = 'The Canvas';
	readonly subtitle = 'Forge doesn\'t have a code editor with AI bolted on. The canvas is shared — AI chat and code live as equals.';

	private _selectedLayout: CanvasLayout = 'quad';
	private _previewEl?: HTMLElement;

	get selectedLayout(): CanvasLayout {
		return this._selectedLayout;
	}

	render(container: HTMLElement, _env: IEnvironmentDetectionResult): void {
		const body = document.createElement('div');
		body.className = 'forge-onboarding-body';

		const preview = document.createElement('div');
		preview.className = 'forge-onboarding-quad-preview';
		this._previewEl = preview;
		this._renderPreview(this._selectedLayout);
		body.appendChild(preview);

		// Layout selector — 4 cards below the preview
		const grid = document.createElement('div');
		grid.className = 'forge-onboarding-layout-grid';

		const cardEls = new Map<CanvasLayout, HTMLElement>();
		for (const layout of LAYOUTS) {
			const card = this._buildLayoutCard(layout);
			cardEls.set(layout.id, card);
			card.addEventListener('click', () => {
				this._selectedLayout = layout.id;
				for (const [id, el] of cardEls) {
					el.classList.toggle('selected', id === layout.id);
				}
				this._renderPreview(layout.id);
			});
			grid.appendChild(card);
		}

		body.appendChild(grid);

		const requiredTag = document.createElement('div');
		requiredTag.className = 'forge-onboarding-required-tag';
		requiredTag.textContent = 'Cannot be skipped';
		body.appendChild(requiredTag);

		container.appendChild(body);
	}

	validate(): boolean {
		return true;
	}

	async onNext(): Promise<void> {
		// no-op — selection is read by Step4Ready via selectedLayout getter
	}

	private _renderPreview(layoutId: CanvasLayout): void {
		const preview = this._previewEl;
		if (!preview) {
			return;
		}
		const def = LAYOUTS.find(l => l.id === layoutId);
		if (!def) {
			return;
		}

		preview.replaceChildren();
		preview.classList.remove('layout-focus', 'layout-split', 'layout-quad', 'layout-code-ai');
		preview.classList.add(`layout-${layoutId}`);

		for (const pane of def.panes) {
			preview.appendChild(this._buildPane(pane));
		}
	}

	private _buildPane(pane: PaneSpec): HTMLElement {
		const paneEl = document.createElement('div');
		paneEl.className = 'forge-onboarding-pane';

		const dot = document.createElement('div');
		dot.className = `forge-onboarding-pane-dot forge-onboarding-pane-dot--${pane.dotClass}`;
		paneEl.appendChild(dot);

		const label = document.createElement('div');
		label.className = 'forge-onboarding-pane-label';
		label.textContent = pane.label;

		if (pane.cursor) {
			const cursor = document.createElement('span');
			cursor.className = 'forge-onboarding-pane-cursor';
			label.appendChild(cursor);
		}

		paneEl.appendChild(label);
		return paneEl;
	}

	private _buildLayoutCard(layout: LayoutDefinition): HTMLElement {
		const card = document.createElement('div');
		card.className = 'forge-onboarding-layout-card' + (layout.id === this._selectedLayout ? ' selected' : '');

		const preview = document.createElement('div');
		preview.className = `forge-onboarding-layout-preview-mini ${layout.id}`;
		for (let i = 0; i < layout.panes.length; i++) {
			const cell = document.createElement('div');
			cell.className = 'forge-onboarding-layout-preview-cell';
			preview.appendChild(cell);
		}
		card.appendChild(preview);

		const label = document.createElement('div');
		label.className = 'forge-onboarding-layout-label';
		label.textContent = layout.label;
		card.appendChild(label);

		return card;
	}
}
