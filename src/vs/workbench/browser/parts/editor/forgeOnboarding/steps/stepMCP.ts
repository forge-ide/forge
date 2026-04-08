/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { IForgeOnboardingService, IEnvironmentDetectionResult } from '../../../../../services/forge/common/forgeOnboardingService.js';
import { IOnboardingStep } from '../forgeOnboardingView.js';

interface MCPServerDefinition {
	id: string;
	label: string;
	description: string;
	iconText: string;
	recommended: boolean;
}

const MCP_SERVERS: MCPServerDefinition[] = [
	{ id: 'filesystem', label: 'Filesystem MCP', description: 'Read, write, and navigate local files - required for most coding tasks', iconText: '[fs]', recommended: true },
	{ id: 'github', label: 'GitHub MCP', description: 'Search repos, create PRs, review issues - requires GitHub token', iconText: '[gh]', recommended: false },
	{ id: 'browser', label: 'Browser MCP', description: 'Headless browser automation via Playwright', iconText: '[br]', recommended: false },
	{ id: 'postgres', label: 'Postgres MCP', description: 'Query and inspect PostgreSQL databases', iconText: '[pg]', recommended: false },
];

export class StepMCP implements IOnboardingStep {
	readonly stepId = 'mcp';
	readonly title = 'MCP Servers';
	readonly subtitle = 'MCP servers let your AI read and write files, search code, query databases, and more.';

	private _selected: Set<string> = new Set(['filesystem', 'github']);

	constructor(private readonly _onboardingService: IForgeOnboardingService) { }

	get selectedServers(): string[] {
		return MCP_SERVERS.map(s => s.id).filter(id => this._selected.has(id));
	}

	toggleServer(id: string): void {
		if (this._selected.has(id)) {
			this._selected.delete(id);
		} else {
			this._selected.add(id);
		}
	}

	render(container: HTMLElement, env: IEnvironmentDetectionResult): void {
		const body = document.createElement('div');
		body.className = 'forge-onboarding-body';

		if (env.npxAvailable) {
			const banner = document.createElement('div');
			banner.className = 'forge-onboarding-detect found';

			const dot = document.createElement('div');
			dot.className = 'forge-onboarding-detect-dot';
			banner.appendChild(dot);

			banner.appendChild(document.createTextNode('npx available - MCP servers can be launched automatically'));
			body.appendChild(banner);
		}

		const list = document.createElement('div');
		list.className = 'forge-onboarding-mcp-list';

		for (const server of MCP_SERVERS) {
			list.appendChild(this._buildServerOption(server));
		}

		body.appendChild(list);

		const note = document.createElement('div');
		note.className = 'forge-onboarding-mcp-note';
		note.textContent = 'More MCP servers available in the plugin registry after setup.';
		body.appendChild(note);

		container.appendChild(body);
	}

	private _buildServerOption(server: MCPServerDefinition): HTMLElement {
		const option = document.createElement('div');
		option.className = 'forge-onboarding-mcp-option' + (this._selected.has(server.id) ? ' selected' : '');
		option.dataset['serverId'] = server.id;

		const icon = document.createElement('span');
		icon.className = 'forge-onboarding-mcp-icon';
		icon.textContent = server.iconText;
		option.appendChild(icon);

		const info = document.createElement('div');
		info.className = 'forge-onboarding-mcp-info';

		const label = document.createElement('div');
		label.className = 'forge-onboarding-mcp-label';
		label.textContent = server.label;
		info.appendChild(label);

		const desc = document.createElement('div');
		desc.className = 'forge-onboarding-mcp-desc';
		desc.textContent = server.description;
		info.appendChild(desc);

		option.appendChild(info);

		if (server.recommended) {
			const badge = document.createElement('span');
			badge.className = 'forge-onboarding-mcp-rec-badge';
			badge.textContent = 'recommended';
			option.appendChild(badge);
		}

		// Create check element upfront; show/hide via display.
		// Glyph is rendered via CSS ::before to keep source ASCII-only.
		const checkEl = document.createElement('span');
		checkEl.className = 'forge-onboarding-mcp-check';
		checkEl.style.display = this._selected.has(server.id) ? '' : 'none';
		option.appendChild(checkEl);

		option.addEventListener('click', () => {
			this.toggleServer(server.id);
			const isSelected = this._selected.has(server.id);
			option.classList.toggle('selected', isSelected);
			checkEl.style.display = isSelected ? '' : 'none';
		});

		return option;
	}

	validate(): boolean {
		return true;
	}

	async onNext(): Promise<void> {
		await this._onboardingService.saveMCPSelections(this.selectedServers);
	}
}
