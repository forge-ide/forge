/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import assert from 'assert';
import { Event } from '../../../../../../base/common/event.js';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../../base/test/common/utils.js';
import { CommandsRegistry } from '../../../../../../platform/commands/common/commands.js';
import type { ServicesAccessor } from '../../../../../../platform/instantiation/common/instantiation.js';
import { ForgeLayout, IForgeLayoutService } from '../../../../../services/forge/common/forgeLayoutService.js';

// Import the contribution module to trigger command registrations via registerAction2 side effects
import '../../../../../browser/parts/editor/forgeChat/forgeChat.contribution.js';

/**
 * Stub IForgeLayoutService that records setLayout calls.
 */
class StubForgeLayoutService implements IForgeLayoutService {
	declare readonly _serviceBrand: undefined;

	readonly onDidChangeLayout = Event.None;
	readonly setLayoutCalls: ForgeLayout[] = [];

	private _activeLayout: ForgeLayout = 'focus';
	get activeLayout(): ForgeLayout { return this._activeLayout; }

	async setLayout(layout: ForgeLayout): Promise<void> {
		this.setLayoutCalls.push(layout);
		this._activeLayout = layout;
	}

	async openChatPane(): Promise<void> { }
	getLayoutState() { return { layout: this._activeLayout as ForgeLayout, panes: [] }; }
	saveLayout(): void { }
	async restoreLayout(): Promise<void> { }
}

suite('Forge Layout Commands', () => {

	ensureNoDisposablesAreLeakedInTestSuite();

	test('forge.layout.quad command is registered', () => {
		const command = CommandsRegistry.getCommand('forge.layout.quad');
		assert.ok(command, 'forge.layout.quad should be registered in CommandsRegistry');
	});

	test('forge.layout.split command is registered', () => {
		const command = CommandsRegistry.getCommand('forge.layout.split');
		assert.ok(command, 'forge.layout.split should be registered in CommandsRegistry');
	});

	test('forge.layout.focus command is registered', () => {
		const command = CommandsRegistry.getCommand('forge.layout.focus');
		assert.ok(command, 'forge.layout.focus should be registered in CommandsRegistry');
	});

	test('forge.layout.codeai command is registered', () => {
		const command = CommandsRegistry.getCommand('forge.layout.codeai');
		assert.ok(command, 'forge.layout.codeai should be registered in CommandsRegistry');
	});

	test('all four layout commands are registered in the command palette (f1: true)', () => {
		const layoutCommandIds = [
			'forge.layout.quad',
			'forge.layout.split',
			'forge.layout.focus',
			'forge.layout.codeai',
		];

		for (const id of layoutCommandIds) {
			const command = CommandsRegistry.getCommand(id);
			assert.ok(command, `${id} should be registered`);
			// Commands registered via Action2 with f1: true are present in the registry.
			// The metadata.description field is set from the title when f1 is true.
			assert.ok(command.metadata?.description, `${id} should have metadata with a description (f1 palette entry)`);
		}
	});

	test('forge.layout.quad command calls setLayout("quad") on ForgeLayoutService', async () => {
		const stub = new StubForgeLayoutService();
		const command = CommandsRegistry.getCommand('forge.layout.quad');
		assert.ok(command);

		// Create a minimal ServicesAccessor that returns the stub for IForgeLayoutService
		const accessor = {
			get<T>(id: { toString(): string }): T {
				if (id === IForgeLayoutService) {
					return stub as unknown as T;
				}
				throw new Error(`Unexpected service request: ${id}`);
			},
		};

		await command.handler(accessor as unknown as ServicesAccessor);

		assert.strictEqual(stub.setLayoutCalls.length, 1);
		assert.strictEqual(stub.setLayoutCalls[0], 'quad');
	});

	test('forge.layout.split command calls setLayout("split")', async () => {
		const stub = new StubForgeLayoutService();
		const command = CommandsRegistry.getCommand('forge.layout.split');
		assert.ok(command);

		const accessor = {
			get<T>(id: { toString(): string }): T {
				if (id === IForgeLayoutService) {
					return stub as unknown as T;
				}
				throw new Error(`Unexpected service request: ${id}`);
			},
		};

		await command.handler(accessor as unknown as ServicesAccessor);

		assert.strictEqual(stub.setLayoutCalls.length, 1);
		assert.strictEqual(stub.setLayoutCalls[0], 'split');
	});

	test('forge.layout.focus command calls setLayout("focus")', async () => {
		const stub = new StubForgeLayoutService();
		const command = CommandsRegistry.getCommand('forge.layout.focus');
		assert.ok(command);

		const accessor = {
			get<T>(id: { toString(): string }): T {
				if (id === IForgeLayoutService) {
					return stub as unknown as T;
				}
				throw new Error(`Unexpected service request: ${id}`);
			},
		};

		await command.handler(accessor as unknown as ServicesAccessor);

		assert.strictEqual(stub.setLayoutCalls.length, 1);
		assert.strictEqual(stub.setLayoutCalls[0], 'focus');
	});

	test('forge.layout.codeai command calls setLayout("code+ai")', async () => {
		const stub = new StubForgeLayoutService();
		const command = CommandsRegistry.getCommand('forge.layout.codeai');
		assert.ok(command);

		const accessor = {
			get<T>(id: { toString(): string }): T {
				if (id === IForgeLayoutService) {
					return stub as unknown as T;
				}
				throw new Error(`Unexpected service request: ${id}`);
			},
		};

		await command.handler(accessor as unknown as ServicesAccessor);

		assert.strictEqual(stub.setLayoutCalls.length, 1);
		assert.strictEqual(stub.setLayoutCalls[0], 'code+ai');
	});
});
