/**
 * Skill parsing utilities. SkillDefinition type is in forgeConfigResolutionTypes.ts.
 * This file provides the markdown parser for .skills/ files.
 */

import { SkillDefinition } from './forgeConfigResolutionTypes.js';

/**
 * Parse a skill markdown file. Same frontmatter pattern as agents.
 * Returns null if required fields (name) are missing.
 */
export function parseSkillMarkdown(content: string): SkillDefinition | null {
	const frontmatterMatch = content.match(/^---\r?\n([\s\S]*?)\r?\n---\r?\n?([\s\S]*)$/);
	if (!frontmatterMatch) {
		return null;
	}

	const frontmatter = parseYamlFrontmatter(frontmatterMatch[1]);
	const body = frontmatterMatch[2].trim();

	if (!frontmatter.name || typeof frontmatter.name !== 'string') {
		return null;
	}

	return {
		name: frontmatter.name,
		description: typeof frontmatter.description === 'string' ? frontmatter.description : '',
		content: body,
	};
}

function parseYamlFrontmatter(yaml: string): Record<string, unknown> {
	const result: Record<string, unknown> = {};
	for (const line of yaml.split('\n')) {
		const match = line.match(/^(\w+):\s*(.+)$/);
		if (!match) {
			continue;
		}
		const [, key, rawValue] = match;
		const value = rawValue.trim();
		result[key] = value.replace(/^["']|["']$/g, '');
	}
	return result;
}
