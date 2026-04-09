/*---------------------------------------------------------------------------------------------
 * Forge - ForgeSkillsView DOM helpers
 *--------------------------------------------------------------------------------------------*/

import { SkillDefinition } from '../../../services/forge/common/forgeConfigResolutionTypes.js';

export function createSkillRow(skill: SkillDefinition): HTMLElement {
	const row = document.createElement('div');
	row.className = 'forge-skill-row';
	row.dataset['name'] = skill.name;

	const name = document.createElement('div');
	name.className = 'forge-skill-row-name';
	name.textContent = skill.name;
	row.appendChild(name);

	if (skill.description) {
		const desc = document.createElement('div');
		desc.className = 'forge-skill-row-desc';
		desc.textContent = skill.description;
		row.appendChild(desc);
	}

	return row;
}

export function renderSkillPreview(container: HTMLElement, skill: SkillDefinition): void {
	container.innerHTML = '';

	const title = document.createElement('div');
	title.className = 'forge-detail-title';
	title.textContent = skill.name;
	container.appendChild(title);

	if (skill.sourcePath) {
		const pathEl = document.createElement('div');
		pathEl.className = 'forge-skill-source-path';
		pathEl.textContent = skill.sourcePath;
		pathEl.title = skill.sourcePath;
		container.appendChild(pathEl);
	}

	const preview = document.createElement('pre');
	preview.className = 'forge-skill-preview';
	preview.textContent = skill.content ?? '';
	container.appendChild(preview);
}
