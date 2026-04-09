/*---------------------------------------------------------------------------------------------
 * Forge - ForgeSkillService (browser)
 *--------------------------------------------------------------------------------------------*/

import { Emitter, Event } from '../../../../base/common/event.js';
import { Disposable } from '../../../../base/common/lifecycle.js';
import { URI } from '../../../../base/common/uri.js';
import { IFileService } from '../../../../platform/files/common/files.js';
import { IWorkspaceContextService } from '../../../../platform/workspace/common/workspace.js';
import { ILogService } from '../../../../platform/log/common/log.js';
import { IForgeSkillService } from '../common/forgeSkillService.js';
import { SkillDefinition } from '../common/forgeConfigResolutionTypes.js';
import { parseSkillMarkdown } from '../common/forgeSkillTypes.js';

export class ForgeSkillService extends Disposable implements IForgeSkillService {
	declare readonly _serviceBrand: undefined;

	private _skills: SkillDefinition[] = [];
	private readonly _onDidChangeSkills = this._register(new Emitter<void>());
	readonly onDidChangeSkills: Event<void> = this._onDidChangeSkills.event;

	constructor(
		@IFileService private readonly _fileService: IFileService,
		@IWorkspaceContextService private readonly _workspaceService: IWorkspaceContextService,
		@ILogService private readonly _logService: ILogService,
	) {
		super();
		this._loadSkills();
		this._register(this._fileService.onDidFilesChange(() => {
			this._loadSkills();
			this._onDidChangeSkills.fire();
		}));
	}

	getAvailableSkills(): SkillDefinition[] {
		return this._skills;
	}

	private async _loadSkills(): Promise<void> {
		const folders = this._workspaceService.getWorkspace().folders;
		if (folders.length === 0) { return; }

		const root = folders[0].uri;
		const skillsDir = URI.joinPath(root, '.forge-agents', 'skills');

		try {
			const children = await this._fileService.resolve(skillsDir);
			if (!children.children) { return; }

			const skills: SkillDefinition[] = [];
			for (const child of children.children) {
				if (!child.name.endsWith('.md')) { continue; }
				try {
					const content = await this._fileService.readFile(child.resource);
					const text = content.value.toString();
					const parsed = parseSkillMarkdown(text);
					if (parsed) {
						skills.push({ ...parsed, sourcePath: child.resource.fsPath });
					}
				} catch (e) {
					this._logService.warn(`ForgeSkillService: failed to parse ${child.name}`, e);
				}
			}
			this._skills = skills;
		} catch {
			// Directory doesn't exist — no skills available
			this._skills = [];
		}
	}
}
