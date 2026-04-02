/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import type { IAIProvider } from './aiProvider.js';

/**
 * In-memory registry mapping provider names to their IAIProvider instances.
 * Used internally by IAIProviderService implementations — not a DI service itself.
 */
export class ProviderRegistry {
	private readonly providers = new Map<string, IAIProvider>();

	register(name: string, provider: IAIProvider): void {
		this.providers.set(name, provider);
	}

	unregister(name: string): boolean {
		return this.providers.delete(name);
	}

	has(name: string): boolean {
		return this.providers.has(name);
	}

	resolve(name: string): IAIProvider | undefined {
		return this.providers.get(name);
	}

	list(): string[] {
		return Array.from(this.providers.keys());
	}
}
