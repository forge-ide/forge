/*---------------------------------------------------------------------------------------------
 * Forge - IForgeSessionContextService
 *--------------------------------------------------------------------------------------------*/

import { Event } from '../../../../base/common/event.js';
import { createDecorator } from '../../../../platform/instantiation/common/instantiation.js';

export const IForgeSessionContextService = createDecorator<IForgeSessionContextService>('forgeSessionContextService');

export interface IForgeSessionContext {
	readonly sessionId: string;
	readonly activeSkills: string[];
	readonly activeMcpServers: string[];
}

export interface IForgeSessionContextService {
	readonly _serviceBrand: undefined;
	/** Fires with the sessionId whenever a context changes. */
	readonly onDidChangeContext: Event<string>;
	getContext(sessionId: string): IForgeSessionContext;
	setSkills(sessionId: string, skills: string[]): void;
	setMcpServers(sessionId: string, servers: string[]): void;
	clearContext(sessionId: string): void;
}
