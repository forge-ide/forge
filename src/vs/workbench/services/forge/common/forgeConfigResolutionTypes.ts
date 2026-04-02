/**
 * Portable, tool-agnostic configuration types.
 * These types map to ecosystem-standard file formats (.mcp.json, .agents/, .skills/)
 * and are not Forge-specific — any agent tool can read these files.
 */

// --- .mcp.json types (ecosystem standard) ---

export interface McpServerEntry {
	readonly name: string;
	/** stdio transport */
	readonly command?: string;
	readonly args?: string[];
	readonly env?: Record<string, string>;
	/** HTTP/SSE transport */
	readonly url?: string;
	readonly headers?: Record<string, string>;
}

export interface McpJsonConfig {
	readonly mcpServers: Record<string, Omit<McpServerEntry, 'name'>>;
}

export interface McpJsonParseResult {
	readonly valid: boolean;
	readonly servers: McpServerEntry[];
	readonly error?: string;
}

export function parseMcpJson(raw: unknown): McpJsonParseResult {
	if (!raw || typeof raw !== 'object') {
		return { valid: false, servers: [], error: 'Input must be an object' };
	}

	const obj = raw as Record<string, unknown>;
	if (!obj.mcpServers || typeof obj.mcpServers !== 'object') {
		return { valid: false, servers: [], error: 'Missing "mcpServers" key' };
	}

	const serversObj = obj.mcpServers as Record<string, Record<string, unknown>>;
	const servers: McpServerEntry[] = [];

	for (const [name, config] of Object.entries(serversObj)) {
		servers.push({
			name,
			command: config.command as string | undefined,
			args: config.args as string[] | undefined,
			env: config.env as Record<string, string> | undefined,
			url: config.url as string | undefined,
			headers: config.headers as Record<string, string> | undefined,
		});
	}

	return { valid: true, servers };
}

// --- .agents/ types ---

export interface AgentDefinition {
	readonly name: string;
	readonly description: string;
	readonly systemPrompt: string;
	readonly tools?: string[];
	readonly maxTurns?: number;
	readonly provider?: string;
	readonly model?: string;
	/** Filesystem path the definition was loaded from */
	readonly sourcePath?: string;
}

export interface AgentsJsonConfig {
	readonly defaults?: {
		readonly maxTurns?: number;
		readonly provider?: string;
		readonly model?: string;
	};
}

/**
 * Parse a markdown file with YAML frontmatter into an AgentDefinition.
 * Returns null if the file is missing required fields (name).
 */
export function parseAgentMarkdown(content: string): AgentDefinition | null {
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
		systemPrompt: body,
		tools: Array.isArray(frontmatter.tools) ? frontmatter.tools : undefined,
		maxTurns: typeof frontmatter.maxTurns === 'number' ? frontmatter.maxTurns : undefined,
		provider: typeof frontmatter.provider === 'string' ? frontmatter.provider : undefined,
		model: typeof frontmatter.model === 'string' ? frontmatter.model : undefined,
	};
}

/**
 * Minimal YAML frontmatter parser. Handles simple key: value pairs,
 * inline arrays [a, b, c], and numeric values. Not a full YAML parser.
 */
export function parseYamlFrontmatter(yaml: string): Record<string, unknown> {
	const result: Record<string, unknown> = {};
	for (const line of yaml.split('\n')) {
		const match = line.match(/^(\w+):\s*(.+)$/);
		if (!match) {
			continue;
		}
		const [, key, rawValue] = match;
		const value = rawValue.trim();

		// Inline array: [a, b, c]
		if (value.startsWith('[') && value.endsWith(']')) {
			result[key] = value.slice(1, -1).split(',').map(s => s.trim()).filter(Boolean);
			continue;
		}

		// Number
		if (/^\d+$/.test(value)) {
			result[key] = parseInt(value, 10);
			continue;
		}

		// String (strip surrounding quotes if present)
		result[key] = value.replace(/^["']|["']$/g, '');
	}
	return result;
}

// --- .skills/ types ---

export interface SkillDefinition {
	readonly name: string;
	readonly description: string;
	readonly content: string;
	/** Filesystem path the definition was loaded from */
	readonly sourcePath?: string;
}

export interface SkillsJsonConfig {
	readonly defaults?: Record<string, unknown>;
}

// --- forge.json extensions ---

export interface ConfigPaths {
	readonly mcp?: string[];
	readonly agents?: string[];
	readonly skills?: string[];
}

export interface DisabledConfig {
	readonly mcpServers: string[];
	readonly agents: string[];
}

// --- Resolved (merged) config ---

export interface ResolvedConfig {
	readonly mcpServers: McpServerEntry[];
	readonly agents: AgentDefinition[];
	readonly skills: SkillDefinition[];
	readonly disabled: DisabledConfig;
}
