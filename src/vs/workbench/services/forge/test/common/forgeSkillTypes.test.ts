import assert from 'assert';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import { parseSkillMarkdown } from '../../common/forgeSkillTypes.js';
import { SkillDefinition } from '../../common/forgeConfigResolutionTypes.js';

suite('ForgeSkillTypes', () => {
	ensureNoDisposablesAreLeakedInTestSuite();

	test('parseSkillMarkdown extracts frontmatter and body', () => {
		const content = [
			'---',
			'name: summarize-pr',
			'description: Summarizes a pull request',
			'---',
			'',
			'Given a PR diff, produce a concise summary of changes.',
		].join('\n');
		const result = parseSkillMarkdown(content);
		assert.ok(result);
		assert.strictEqual(result.name, 'summarize-pr');
		assert.strictEqual(result.description, 'Summarizes a pull request');
		assert.strictEqual(result.content, 'Given a PR diff, produce a concise summary of changes.');
	});

	test('parseSkillMarkdown returns null for missing frontmatter', () => {
		const result = parseSkillMarkdown('Just text, no frontmatter');
		assert.strictEqual(result, null);
	});

	test('parseSkillMarkdown returns null for missing name', () => {
		const content = [
			'---',
			'description: No name field',
			'---',
			'body',
		].join('\n');
		const result = parseSkillMarkdown(content);
		assert.strictEqual(result, null);
	});

	test('parseSkillMarkdown handles minimal frontmatter', () => {
		const content = [
			'---',
			'name: minimal-skill',
			'---',
			'Do the thing.',
		].join('\n');
		const result = parseSkillMarkdown(content);
		assert.ok(result);
		assert.strictEqual(result.name, 'minimal-skill');
		assert.strictEqual(result.description, '');
		assert.strictEqual(result.content, 'Do the thing.');
	});

	test('SkillDefinition type assignment', () => {
		const skill: SkillDefinition = {
			name: 'test-skill',
			description: 'A test skill',
			content: 'skill body content',
			sourcePath: '/home/user/.skills/test-skill.md'
		};
		assert.strictEqual(skill.name, 'test-skill');
		assert.strictEqual(skill.sourcePath, '/home/user/.skills/test-skill.md');
	});
});
