/*---------------------------------------------------------------------------------------------
 * Forge - IForgeSkillService
 *--------------------------------------------------------------------------------------------*/

import { Event } from '../../../../base/common/event.js';
import { createDecorator } from '../../../../platform/instantiation/common/instantiation.js';
import { SkillDefinition } from './forgeConfigResolutionTypes.js';

export const IForgeSkillService = createDecorator<IForgeSkillService>('forgeSkillService');

export interface IForgeSkillService {
	readonly _serviceBrand: undefined;
	/** Fires when the set of available skills changes (files added/removed/changed). */
	readonly onDidChangeSkills: Event<void>;
	getAvailableSkills(): SkillDefinition[];
}
