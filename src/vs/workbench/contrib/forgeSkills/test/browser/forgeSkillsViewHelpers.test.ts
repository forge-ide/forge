/*---------------------------------------------------------------------------------------------
 * Forge - ForgeSkillsView helper tests
 *--------------------------------------------------------------------------------------------*/

import * as assert from 'assert';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import { createSkillRow, renderSkillPreview } from '../../browser/forgeSkillsViewHelpers.js';
import { SkillDefinition } from '../../../../services/forge/common/forgeConfigResolutionTypes.js';

function makeSkill(overrides: Partial<SkillDefinition> = {}): SkillDefinition {
	return { name: 'commit', description: 'Create git commits', content: '---\nname: commit\n---\nBody text', ...overrides };
}

suite('forgeSkillsViewHelpers', () => {
	ensureNoDisposablesAreLeakedInTestSuite();

	suite('createSkillRow', () => {
		test('renders skill name', () => {
			const row = createSkillRow(makeSkill({ name: 'commit' }));
			assert.ok(row.textContent?.includes('commit'));
		});

		test('renders skill description', () => {
			const row = createSkillRow(makeSkill({ description: 'Create git commits' }));
			assert.ok(row.textContent?.includes('Create git commits'));
		});

		test('sets data-name attribute', () => {
			const row = createSkillRow(makeSkill({ name: 'tdd' }));
			assert.strictEqual(row.dataset['name'], 'tdd');
		});
	});

	suite('renderSkillPreview', () => {
		test('renders skill content', () => {
			const container = document.createElement('div');
			renderSkillPreview(container, makeSkill({ content: 'Body text here' }));
			assert.ok(container.textContent?.includes('Body text here'));
		});

		test('renders source path when present', () => {
			const container = document.createElement('div');
			renderSkillPreview(container, makeSkill({ sourcePath: '/home/user/.forge-agents/skills/commit.md' }));
			assert.ok(container.textContent?.includes('commit.md'));
		});
	});
});
