/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { IEnvironmentDetectionResult } from '../../../../../services/forge/common/forgeOnboardingService.js';
import { IOnboardingStep } from '../forgeOnboardingView.js';

const SVG_NS = 'http://www.w3.org/2000/svg';

// Stroke-only line icons rendered at 18px in the import toggle rows. Dots use the
// `v.01` trick — a near-zero line with round linecap renders as a 1.5px round dot.
const ICON_PATHS: Record<string, string[]> = {
	keyboard: [
		'M3 6h18v12H3z',
		'M6 10v.01M9 10v.01M12 10v.01M15 10v.01M18 10v.01',
		'M6 13v.01M9 13v.01M12 13v.01M15 13v.01M18 13v.01',
		'M8 16h8',
	],
	window: [
		'M3 5h18v14H3z',
		'M3 9h18',
		'M8 9v10',
	],
	grid: [
		'M4 4h7v7H4z',
		'M13 4h7v7h-7z',
		'M4 13h7v7H4z',
		'M13 13h7v7h-7z',
	],
	branch: [
		'M6 3v18',
		'M18 9v12',
		'M6 12c0-3 6-3 12-3',
	],
	cpu: [
		'M6 6h12v12H6z',
		'M9 9h6v6H9z',
		'M2 9h2', 'M2 14h2', 'M20 9h2', 'M20 14h2',
		'M9 2v2', 'M14 2v2', 'M9 20v2', 'M14 20v2',
	],
};

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

export class Step2Import implements IOnboardingStep {
	readonly stepId = 'import';
	readonly title = 'IMPORT YOUR CONFIG';
	readonly subtitle = 'We found an existing VS Code setup. Choose what to bring across.';

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

		const toggleDefs: Array<{ icon: string; label: string; meta: string; field: keyof Step2Import; defaultOn: boolean }> = [
			{ icon: 'keyboard', label: 'Keybindings', meta: 'Custom keybindings.json detected', field: 'importKeybindings', defaultOn: true },
			{ icon: 'window', label: 'Theme & UI Settings', meta: 'Font size, line height, minimap, sidebar position', field: 'importTheme', defaultOn: true },
			{ icon: 'grid', label: 'Extensions', meta: 'Compatible extensions will be installed', field: 'importExtensions', defaultOn: true },
			{ icon: 'branch', label: 'Git settings', meta: 'user.name, user.email, default branch', field: 'importGit', defaultOn: true },
			{ icon: 'cpu', label: 'Copilot / AI extension config', meta: 'Will be replaced by Forge\'s native AI system', field: 'importCopilotConfig', defaultOn: false },
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

	private _toggleRow(iconKey: string, label: string, meta: string, initialOn: boolean, onChange: (on: boolean) => void): HTMLElement {
		let state = initialOn;

		const row = document.createElement('div');
		row.className = 'forge-onboarding-toggle-row';

		const iconEl = document.createElement('span');
		iconEl.className = 'forge-onboarding-toggle-icon';
		iconEl.appendChild(createSvgIcon(ICON_PATHS[iconKey] ?? []));
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
