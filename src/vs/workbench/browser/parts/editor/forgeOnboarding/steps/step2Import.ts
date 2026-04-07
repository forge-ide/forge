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

	importKeybindings = true;
	importTheme = true;
	importExtensions = true;
	importGit = true;
	importCopilotConfig = false; // OFF by default — replaced by Forge's native AI

	render(container: HTMLElement, env: IEnvironmentDetectionResult): void {
		const body = document.createElement('div');
		body.className = 'forge-onboarding-body';

		if (!env.hasVSCodeConfig) {
			const msg = document.createElement('p');
			msg.className = 'forge-onboarding-no-config';
			msg.textContent = 'No VS Code config found — you can configure Forge from scratch.';
			body.appendChild(msg);
			container.appendChild(body);
			return;
		}

		const subtitle = document.createElement('p');
		subtitle.className = 'forge-onboarding-subtitle';
		subtitle.textContent = 'We found an existing VS Code setup. Choose what to bring across...';
		body.appendChild(subtitle);

		const toggleDefs: Array<{ icon: string; label: string; meta: string; field: keyof Step2Import; defaultOn: boolean }> = [
			{ icon: '[key]', label: 'Keybindings', meta: 'Custom keybindings.json detected', field: 'importKeybindings', defaultOn: true },
			{ icon: '[ui]', label: 'Theme & UI Settings', meta: 'Font size, line height, minimap, sidebar position', field: 'importTheme', defaultOn: true },
			{ icon: '[ext]', label: 'Extensions', meta: 'Compatible extensions will be installed', field: 'importExtensions', defaultOn: true },
			{ icon: '[git]', label: 'Git settings', meta: 'user.name, user.email, default branch', field: 'importGit', defaultOn: true },
			{ icon: '[ai]', label: 'Copilot / AI extension config', meta: 'Will be replaced by Forge\'s native AI system', field: 'importCopilotConfig', defaultOn: false },
		];

		const list = document.createElement('div');
		list.className = 'forge-onboarding-toggle-list';

		for (const def of toggleDefs) {
			const currentValue = this[def.field] as boolean;
			list.appendChild(this._toggleRow(def.icon, def.label, def.meta, currentValue, (checked) => {
				(this[def.field] as boolean) = checked;
			}));
		}

		body.appendChild(list);
		container.appendChild(body);
	}

	validate(): boolean {
		return true;
	}

	async onNext(): Promise<void> {
		// import happens in ForgeOnboardingView after step completes
	}

	private _toggleRow(icon: string, label: string, meta: string, initialOn: boolean, onChange: (on: boolean) => void): HTMLElement {
		let state = initialOn;

		const row = document.createElement('div');
		row.className = 'forge-onboarding-toggle-row';

		const iconEl = document.createElement('span');
		iconEl.className = 'forge-onboarding-toggle-icon';
		iconEl.textContent = icon;
		row.appendChild(iconEl);

		const text = document.createElement('div');
		text.className = 'forge-onboarding-toggle-text';

		const labelEl = document.createElement('div');
		labelEl.className = 'forge-onboarding-toggle-label';
		labelEl.textContent = label;
		text.appendChild(labelEl);

		const metaEl = document.createElement('div');
		metaEl.className = 'forge-onboarding-toggle-meta';
		metaEl.textContent = meta;
		text.appendChild(metaEl);

		row.appendChild(text);

		const toggle = document.createElement('div');
		toggle.className = `forge-onboarding-toggle ${state ? 'on' : 'off'}`;

		const thumb = document.createElement('div');
		thumb.className = 'forge-onboarding-toggle-thumb';
		toggle.appendChild(thumb);

		row.appendChild(toggle);

		row.addEventListener('click', () => {
			state = !state;
			toggle.className = `forge-onboarding-toggle ${state ? 'on' : 'off'}`;
			onChange(state);
		});

		return row;
	}
}
