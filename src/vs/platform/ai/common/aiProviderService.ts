/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { createDecorator } from '../../instantiation/common/instantiation.js';
import { Event } from '../../../base/common/event.js';
import type { IAIProvider } from './aiProvider.js';

export const IAIProviderService = createDecorator<IAIProviderService>('aiProviderService');

export interface IAIProviderService {
	readonly _serviceBrand: undefined;

	/** Fires the list of registered provider names whenever the set changes. */
	readonly onDidChangeProviders: Event<string[]>;

	registerProvider(name: string, provider: IAIProvider): void;
	unregisterProvider(name: string): void;
	getProvider(name: string): IAIProvider | undefined;
	has(name: string): boolean;
	listProviders(): string[];

	/** Returns the name of the default provider from config, or undefined. */
	getDefaultProviderName(): string | undefined;
	setDefaultProviderName(name: string): void;
}
