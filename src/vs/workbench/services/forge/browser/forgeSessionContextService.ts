/*---------------------------------------------------------------------------------------------
 * Forge - ForgeSessionContextService (browser)
 *--------------------------------------------------------------------------------------------*/

import { Emitter, Event } from '../../../../base/common/event.js';
import { Disposable } from '../../../../base/common/lifecycle.js';
import { IForgeSessionContext, IForgeSessionContextService } from '../common/forgeSessionContextService.js';

export class ForgeSessionContextService extends Disposable implements IForgeSessionContextService {
	declare readonly _serviceBrand: undefined;

	private readonly _contexts = new Map<string, { activeSkills: string[]; activeMcpServers: string[] }>();
	private readonly _onDidChangeContext = this._register(new Emitter<string>());
	readonly onDidChangeContext: Event<string> = this._onDidChangeContext.event;

	getContext(sessionId: string): IForgeSessionContext {
		const ctx = this._contexts.get(sessionId);
		return {
			sessionId,
			activeSkills: ctx?.activeSkills ?? [],
			activeMcpServers: ctx?.activeMcpServers ?? [],
		};
	}

	setSkills(sessionId: string, skills: string[]): void {
		const ctx = this._contexts.get(sessionId) ?? { activeSkills: [], activeMcpServers: [] };
		this._contexts.set(sessionId, { ...ctx, activeSkills: skills });
		this._onDidChangeContext.fire(sessionId);
	}

	setMcpServers(sessionId: string, servers: string[]): void {
		const ctx = this._contexts.get(sessionId) ?? { activeSkills: [], activeMcpServers: [] };
		this._contexts.set(sessionId, { ...ctx, activeMcpServers: servers });
		this._onDidChangeContext.fire(sessionId);
	}

	clearContext(sessionId: string): void {
		this._contexts.delete(sessionId);
	}
}
