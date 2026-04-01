/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import assert from 'assert';
import { Event } from '../../../../../../base/common/event.js';
import { DisposableStore } from '../../../../../../base/common/lifecycle.js';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../../base/test/common/utils.js';
import { ForgeChatInput } from '../../../../../browser/parts/editor/forgeChat/forgeChatInput.js';
import type { PanePosition } from '../../../../../services/forge/common/forgeLayoutService.js';

suite('ForgeChatInput (enhanced)', () => {

	let disposables: DisposableStore;

	setup(() => {
		disposables = new DisposableStore();
	});

	teardown(() => {
		disposables.dispose();
	});

	ensureNoDisposablesAreLeakedInTestSuite();

	test('setting panePosition updates panePosition getter', () => {
		const input = disposables.add(new ForgeChatInput('anthropic', 'conv-e01'));

		assert.strictEqual(input.panePosition, undefined, 'panePosition should start undefined');

		input.panePosition = 'tl';
		assert.strictEqual(input.panePosition, 'tl');

		input.panePosition = 'br';
		assert.strictEqual(input.panePosition, 'br');
	});

	test('getName includes pane position when set', () => {
		const input = disposables.add(new ForgeChatInput('anthropic', 'conv-e02'));
		input.panePosition = 'tr';

		const name = input.getName();
		assert.ok(name.includes('anthropic'), `Expected name "${name}" to include provider`);
		assert.ok(name.includes('Top Right'), `Expected name "${name}" to include pane position label`);
	});

	test('getName without pane position shows only provider name', () => {
		const input = disposables.add(new ForgeChatInput('openai', 'conv-e03'));

		const name = input.getName();
		assert.ok(name.includes('openai'), `Expected name "${name}" to include provider`);
		// Should not contain any pane position label when unset
		assert.ok(!name.includes('Top Left') && !name.includes('Top Right') &&
			!name.includes('Bottom Left') && !name.includes('Bottom Right'),
			`Expected name "${name}" to not include a pane position label`);
	});

	test('setModel updates model and fires onDidChangeLabel', async () => {
		const input = disposables.add(new ForgeChatInput('anthropic', 'conv-e04'));

		const labelChanged = Event.toPromise(input.onDidChangeLabel);
		input.setModel('claude-sonnet-4-6');

		await labelChanged;
		assert.strictEqual(input.model, 'claude-sonnet-4-6');
	});

	test('setModel with same value does not fire event', () => {
		const input = disposables.add(new ForgeChatInput('anthropic', 'conv-e05'));
		input.setModel('gpt-4o');

		let fired = false;
		disposables.add(input.onDidChangeLabel(() => { fired = true; }));

		input.setModel('gpt-4o');
		assert.strictEqual(fired, false, 'onDidChangeLabel should not fire when model is unchanged');
	});

	test('serialization roundtrip preserves panePosition and model', () => {
		const original = disposables.add(new ForgeChatInput('anthropic', 'conv-e06'));
		original.panePosition = 'bl';
		original.setModel('claude-sonnet-4-6');

		// Serialize using the enhanced shape
		const serialized = JSON.stringify({
			providerName: original.providerName,
			conversationId: original.conversationId,
			panePosition: original.panePosition,
			model: original.model,
		});

		const data = JSON.parse(serialized) as {
			providerName: string;
			conversationId: string;
			panePosition?: string;
			model?: string;
		};

		const restored = disposables.add(new ForgeChatInput(data.providerName, data.conversationId));
		if (data.panePosition) {
			restored.panePosition = data.panePosition as PanePosition;
		}
		if (data.model) {
			restored.setModel(data.model);
		}

		assert.strictEqual(restored.providerName, original.providerName);
		assert.strictEqual(restored.conversationId, original.conversationId);
		assert.strictEqual(restored.panePosition, original.panePosition);
		assert.strictEqual(restored.model, original.model);
		assert.strictEqual(original.matches(restored), true);
	});
});
