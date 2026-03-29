/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { createDecorator } from '../../instantiation/common/instantiation.js';
import { Event } from '../../../base/common/event.js';
import { IAIProvider } from './aiProvider.js';

export const IAIProviderService = createDecorator<IAIProviderService>('aiProviderService');

export interface IAIProviderService {
	readonly _serviceBrand: undefined;
	/** Fires the name of the newly active provider when it changes. */
	readonly onDidChangeProvider: Event<string>;

	registerProvider(name: string, provider: IAIProvider): void;
	getProvider(name: string): IAIProvider | undefined;
	/** Returns the active provider, or undefined if none has been set. */
	getActiveProvider(): IAIProvider | undefined;
	setActiveProvider(name: string): void;
	listProviders(): string[];
}
