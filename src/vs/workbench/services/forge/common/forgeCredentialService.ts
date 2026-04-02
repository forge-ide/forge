/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { Event } from '../../../../base/common/event.js';
import { createDecorator } from '../../../../platform/instantiation/common/instantiation.js';

export const IForgeCredentialService = createDecorator<IForgeCredentialService>('forgeCredentialService');

export interface IForgeCredentialService {
	readonly _serviceBrand: undefined;

	/** Fires the provider name whose credential changed. */
	readonly onDidChangeCredential: Event<string>;

	/**
	 * Retrieve the API key for the given provider, checking SecretStorage first,
	 * then falling back to the environment variable specified by envKey.
	 */
	getApiKey(providerName: string, envKey: string): Promise<string | undefined>;

	/**
	 * Store an API key for a provider in SecretStorage.
	 */
	setApiKey(providerName: string, apiKey: string): Promise<void>;

	/**
	 * Remove a stored API key for a provider from SecretStorage.
	 */
	deleteApiKey(providerName: string): Promise<void>;

	/**
	 * Check whether a credential is available (SecretStorage or env) without returning the value.
	 */
	hasApiKey(providerName: string, envKey: string): Promise<boolean>;
}
