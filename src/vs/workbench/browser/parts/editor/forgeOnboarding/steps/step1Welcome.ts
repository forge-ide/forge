/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { IEnvironmentDetectionResult } from '../../../../../services/forge/common/forgeOnboardingService.js';
import { IOnboardingStep } from '../forgeOnboardingView.js';

export class Step1Welcome implements IOnboardingStep {
	readonly stepId = 'welcome';
	readonly title = 'Welcome to Forge';
	readonly subtitle = 'Your AI-native development environment';

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
			badges.push(this._badge('VS Code config detected'));
		}

		const keyCount = Object.keys(env.detectedApiKeys).length;
		if (keyCount > 0) {
			badges.push(this._badge(`${keyCount} API key${keyCount > 1 ? 's' : ''} detected`));
		}

		if (env.ollamaRunning) {
			badges.push(this._badge('Ollama running'));
		}

		if (env.lmStudioRunning) {
			badges.push(this._badge('LM Studio running'));
		}

		if (env.npxAvailable) {
			badges.push(this._badge('npx available (MCP servers supported)'));
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
