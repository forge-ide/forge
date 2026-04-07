/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { IEnvironmentDetectionResult } from '../../../../../services/forge/common/forgeOnboardingService.js';
import { IOnboardingStep } from '../forgeOnboardingView.js';

export class Step1Welcome implements IOnboardingStep {
	readonly stepId = 'welcome';
	readonly title = 'FORGE IDE';
	readonly subtitle = 'You\'re running a VSCode fork built around one idea: the AI backing your IDE should be your choice.';

	render(container: HTMLElement, env: IEnvironmentDetectionResult): void {
		const body = document.createElement('p');
		body.className = 'forge-onboarding-body';
		body.textContent = 'Forge brings multi-model AI, a quad canvas, and MCP tool support directly into your development environment. Set up takes less than a minute.';
		container.appendChild(body);

		const badges = this._buildEnvBadges(env);
		if (badges.length > 0) {
			const badgeRow = document.createElement('div');
			badgeRow.className = 'forge-onboarding-env-badges';
			for (const badge of badges) {
				badgeRow.appendChild(badge);
			}
			container.appendChild(badgeRow);
		}
	}

	validate(): boolean {
		return true;
	}

	async onNext(): Promise<void> {
		// no-op
	}

	private _buildEnvBadges(env: IEnvironmentDetectionResult): HTMLElement[] {
		const badges: HTMLElement[] = [];

		if (env.hasVSCodeConfig) {
			badges.push(this._badge('VS Code config found'));
		}

		const keyCount = Object.keys(env.detectedApiKeys).length;
		if (keyCount > 0) {
			const label = keyCount === 1
				? `${Object.keys(env.detectedApiKeys)[0]} API key detected`
				: `${keyCount} API keys detected`;
			badges.push(this._badge(label));
		}

		if (env.ollamaRunning) {
			badges.push(this._badge('Ollama running'));
		}

		if (env.lmStudioRunning) {
			badges.push(this._badge('LM Studio running'));
		}

		if (env.npxAvailable) {
			badges.push(this._badge('npx available'));
		}

		return badges;
	}

	private _badge(text: string): HTMLElement {
		const el = document.createElement('span');
		el.className = 'forge-onboarding-badge';

		const dot = document.createElement('span');
		dot.className = 'forge-onboarding-badge-dot';
		el.appendChild(dot);

		el.appendChild(document.createTextNode(text));
		return el;
	}
}
