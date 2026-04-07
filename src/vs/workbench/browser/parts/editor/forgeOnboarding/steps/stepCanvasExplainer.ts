/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { IEnvironmentDetectionResult } from '../../../../../services/forge/common/forgeOnboardingService.js';
import { IOnboardingStep } from '../forgeOnboardingView.js';

export class StepCanvasExplainer implements IOnboardingStep {
	readonly stepId = 'canvas';
	readonly title = 'The Canvas';
	readonly subtitle = 'Forge doesn\'t have a code editor with AI bolted on. The canvas is shared — AI chat and code live as equals.';

	render(container: HTMLElement, _env: IEnvironmentDetectionResult): void {
		const body = document.createElement('div');
		body.className = 'forge-onboarding-body';

		const preview = document.createElement('div');
		preview.className = 'forge-onboarding-quad-preview';

		const panes: Array<{ dotClass: string; label: string; cursor: boolean }> = [
			{ dotClass: 'forge-onboarding-pane-dot--green', label: 'AI chat', cursor: true },
			{ dotClass: 'forge-onboarding-pane-dot--steel', label: 'AI chat', cursor: false },
			{ dotClass: 'forge-onboarding-pane-dot--amber', label: 'Code', cursor: false },
			{ dotClass: 'forge-onboarding-pane-dot--steel', label: 'AI chat', cursor: false },
		];

		for (const pane of panes) {
			const paneEl = document.createElement('div');
			paneEl.className = 'forge-onboarding-pane';

			const dot = document.createElement('div');
			dot.className = `forge-onboarding-pane-dot ${pane.dotClass}`;
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
			preview.appendChild(paneEl);
		}

		body.appendChild(preview);

		const tag = document.createElement('div');
		tag.className = 'forge-onboarding-required-tag';
		tag.textContent = 'Required · Cannot be skipped';
		body.appendChild(tag);

		container.appendChild(body);
	}

	validate(): boolean {
		return true;
	}

	async onNext(): Promise<void> {
		// no-op
	}
}
