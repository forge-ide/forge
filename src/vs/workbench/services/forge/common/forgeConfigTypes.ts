/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

/**
 * Per-model configuration within a provider block.
 */
export interface ForgeModelConfig {
	readonly id: string;
	readonly maxTokens?: number;
	readonly contextBudget?: number;
}

/**
 * Per-provider configuration block in forge.json.
 */
export interface ForgeProviderConfig {
	readonly name: string;
	readonly baseURL?: string;
	readonly envKey?: string;
	readonly models: ForgeModelConfig[];
}

/**
 * Top-level forge.json configuration shape.
 */
export interface ForgeConfig {
	readonly defaultProvider: string;
	readonly defaultModel?: string;
	readonly stream?: boolean;
	readonly providers: ForgeProviderConfig[];
}

/**
 * Fully resolved model config with all values populated for runtime use.
 */
export interface ResolvedModelConfig {
	readonly providerName: string;
	readonly modelId: string;
	readonly baseURL?: string;
	readonly envKey: string;
	readonly maxTokens: number;
	readonly contextBudget: number;
	readonly stream: boolean;
}

/**
 * Default environment variable names per known provider.
 */
export const PROVIDER_ENV_VARS: Record<string, string> = {
	anthropic: 'ANTHROPIC_API_KEY',
	openai: 'OPENAI_API_KEY',
	google: 'GOOGLE_API_KEY',
};

const DEFAULT_MAX_TOKENS = 4096;
const DEFAULT_CONTEXT_BUDGET = 8000;

/**
 * Find a provider block by name.
 */
export function findProvider(config: ForgeConfig, name: string): ForgeProviderConfig | undefined {
	return config.providers.find(p => p.name === name);
}

/**
 * Find a model within a provider block by id.
 */
export function findModel(provider: ForgeProviderConfig, modelId: string): ForgeModelConfig | undefined {
	return provider.models.find(m => m.id === modelId);
}

/**
 * Resolve a fully populated model config from the raw config tree.
 * Returns undefined if the provider or model cannot be found.
 */
export function resolveModelConfig(
	config: ForgeConfig,
	providerName?: string,
	modelId?: string,
): ResolvedModelConfig | undefined {
	const resolvedProviderName = providerName ?? config.defaultProvider;
	const provider = findProvider(config, resolvedProviderName);
	if (!provider) {
		return undefined;
	}

	const resolvedModelId = modelId ?? provider.models[0]?.id ?? config.defaultModel;
	if (!resolvedModelId) {
		return undefined;
	}

	const model = findModel(provider, resolvedModelId);
	if (!model) {
		return undefined;
	}

	return {
		providerName: resolvedProviderName,
		modelId: resolvedModelId,
		baseURL: provider.baseURL,
		envKey: provider.envKey ?? PROVIDER_ENV_VARS[resolvedProviderName] ?? `${resolvedProviderName.toUpperCase()}_API_KEY`,
		maxTokens: model.maxTokens ?? DEFAULT_MAX_TOKENS,
		contextBudget: model.contextBudget ?? DEFAULT_CONTEXT_BUDGET,
		stream: config.stream ?? true,
	};
}
