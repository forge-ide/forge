/*---------------------------------------------------------------------------------------------
 * Forge - ForgeSkillsView (split-pane: skill catalog list + preview panel)
 *--------------------------------------------------------------------------------------------*/

import { reset } from '../../../../base/browser/dom.js';
import { localize } from '../../../../nls.js';
import { URI } from '../../../../base/common/uri.js';
import { IKeybindingService } from '../../../../platform/keybinding/common/keybinding.js';
import { IContextMenuService } from '../../../../platform/contextview/browser/contextView.js';
import { IConfigurationService } from '../../../../platform/configuration/common/configuration.js';
import { IContextKeyService } from '../../../../platform/contextkey/common/contextkey.js';
import { IInstantiationService } from '../../../../platform/instantiation/common/instantiation.js';
import { IOpenerService } from '../../../../platform/opener/common/opener.js';
import { IThemeService } from '../../../../platform/theme/common/themeService.js';
import { IHoverService } from '../../../../platform/hover/browser/hover.js';
import { ViewPane } from '../../../browser/parts/views/viewPane.js';
import { IViewletViewOptions } from '../../../browser/parts/views/viewsViewlet.js';
import { IViewDescriptorService } from '../../../common/views.js';
import { IForgeSkillService } from '../../../services/forge/common/forgeSkillService.js';
import { SkillDefinition } from '../../../services/forge/common/forgeConfigResolutionTypes.js';
import { createSkillRow, renderSkillPreview } from './forgeSkillsViewHelpers.js';

export const FORGE_SKILLS_VIEW_ID = 'workbench.forgeSkills.mainView';

export class ForgeSkillsView extends ViewPane {

	private _leftPane!: HTMLElement;
	private _rightPane!: HTMLElement;
	private _selectedSkillName: string | undefined;

	constructor(
		options: IViewletViewOptions,
		@IForgeSkillService private readonly _skillService: IForgeSkillService,
		@IOpenerService private readonly _myOpenerService: IOpenerService,
		@IKeybindingService keybindingService: IKeybindingService,
		@IContextMenuService contextMenuService: IContextMenuService,
		@IConfigurationService configurationService: IConfigurationService,
		@IContextKeyService contextKeyService: IContextKeyService,
		@IViewDescriptorService viewDescriptorService: IViewDescriptorService,
		@IInstantiationService instantiationService: IInstantiationService,
		@IOpenerService openerService: IOpenerService,
		@IThemeService themeService: IThemeService,
		@IHoverService hoverService: IHoverService,
	) {
		super(options, keybindingService, contextMenuService, configurationService, contextKeyService, viewDescriptorService, instantiationService, openerService, themeService, hoverService);
	}

	protected override renderBody(container: HTMLElement): void {
		super.renderBody(container);
		container.classList.add('forge-skills-view');

		this._leftPane = document.createElement('div');
		this._leftPane.className = 'forge-split-left';
		container.appendChild(this._leftPane);

		this._rightPane = document.createElement('div');
		this._rightPane.className = 'forge-split-right';
		container.appendChild(this._rightPane);

		this._register(this._skillService.onDidChangeSkills(() => this._renderList()));

		this._renderList();
		this._renderEmptyDetail();
	}

	private _renderList(): void {
		reset(this._leftPane);

		const skills = this._skillService.getAvailableSkills();

		if (skills.length === 0) {
			const empty = document.createElement('div');
			empty.className = 'forge-detail-empty';
			empty.textContent = localize('forgeSkills.noSkills', 'No skills found in .forge-agents/skills/');
			this._leftPane.appendChild(empty);
			return;
		}

		for (const skill of skills) {
			const row = createSkillRow(skill);
			if (this._selectedSkillName === skill.name) {
				row.classList.add('forge-row--selected');
			}
			row.addEventListener('click', () => this._selectSkill(skill));
			this._leftPane.appendChild(row);
		}
	}

	private _selectSkill(skill: SkillDefinition): void {
		this._selectedSkillName = skill.name;
		this._renderList();
		this._renderSkillDetail(skill);
	}

	private _renderEmptyDetail(): void {
		reset(this._rightPane);
		const empty = document.createElement('div');
		empty.className = 'forge-detail-empty';
		empty.textContent = localize('forgeSkills.selectSkill', 'Select a skill to preview its content.');
		this._rightPane.appendChild(empty);
	}

	private _renderSkillDetail(skill: SkillDefinition): void {
		reset(this._rightPane);

		const title = document.createElement('div');
		title.className = 'forge-detail-title';
		title.textContent = skill.name;
		this._rightPane.appendChild(title);

		if (skill.sourcePath) {
			const pathEl = document.createElement('div');
			pathEl.className = 'forge-skill-source-path';
			pathEl.textContent = skill.sourcePath;
			pathEl.title = skill.sourcePath;
			this._rightPane.appendChild(pathEl);

			const editBtn = document.createElement('button');
			editBtn.className = 'forge-btn forge-btn--neutral forge-skill-edit-btn';
			editBtn.textContent = localize('forgeSkills.editSkill', 'Edit Skill');
			editBtn.addEventListener('click', () => this._myOpenerService.open(URI.file(skill.sourcePath!)));
			this._rightPane.appendChild(editBtn);
		}

		const preview = document.createElement('pre');
		preview.className = 'forge-skill-preview';
		preview.textContent = skill.content ?? '';
		this._rightPane.appendChild(preview);
	}

	protected override layoutBody(height: number, width: number): void {
		super.layoutBody(height, width);
	}
}

// Re-export helper for use in tests via the view module if needed
export { renderSkillPreview };
