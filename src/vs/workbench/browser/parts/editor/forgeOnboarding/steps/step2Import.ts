/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { IEnvironmentDetectionResult } from '../../../../../services/forge/common/forgeOnboardingService.js';
import { IOnboardingStep } from '../forgeOnboardingView.js';

export class Step2Import implements IOnboardingStep {
	readonly stepId = 'import';
	readonly title = 'Import VS Code Config';
	readonly subtitle = 'Carry your settings over to Forge';

	importSettings = true;
	importKeybindings = true;
	importExtensions = true;
	importApiKeys = true;

	render(container: HTMLElement, env: IEnvironmentDetectionResult): void {
		if (!env.hasVSCodeConfig) {
			const msg = document.createElement('p');
			msg.className = 'forge-onboarding-no-config';
			msg.textContent = 'No VS Code config found — you can configure Forge from scratch.';
			container.appendChild(msg);
			return;
		}

		if (env.vscodeConfigPath) {
			const pathEl = document.createElement('div');
			pathEl.className = 'forge-onboarding-config-path';
			pathEl.textContent = env.vscodeConfigPath;
			container.appendChild(pathEl);
		}

		const checkboxList = document.createElement('div');
		checkboxList.className = 'forge-onboarding-checkbox-list';

		checkboxList.appendChild(this._checkbox('settings', 'Settings', checked => { this.importSettings = checked; }));
		checkboxList.appendChild(this._checkbox('keybindings', 'Keybindings', checked => { this.importKeybindings = checked; }));
		checkboxList.appendChild(this._checkbox('extensions', 'Extensions list', checked => { this.importExtensions = checked; }));
		checkboxList.appendChild(this._checkbox('apikeys', 'API keys', checked => { this.importApiKeys = checked; }));

		container.appendChild(checkboxList);
	}

	validate(): boolean {
		return true;
	}

	async onNext(): Promise<void> {
		// import happens in step 4 or later
	}

	private _checkbox(id: string, label: string, onChange: (checked: boolean) => void): HTMLElement {
		const row = document.createElement('label');
		row.className = 'forge-onboarding-checkbox-row';
		row.htmlFor = `forge-onboarding-cb-${id}`;

		const input = document.createElement('input');
		input.type = 'checkbox';
		input.id = `forge-onboarding-cb-${id}`;
		input.checked = true;
		input.addEventListener('change', () => onChange(input.checked));

		row.appendChild(input);
		row.appendChild(document.createTextNode(label));

		return row;
	}
}
