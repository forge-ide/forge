/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import assert from 'assert';
import { ForgeChatInput } from '../../../../../browser/parts/editor/forgeChat/forgeChatInput.js';
import { DisposableStore } from '../../../../../../base/common/lifecycle.js';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../../base/test/common/utils.js';

suite('ForgeChatInput', () => {

	let disposables: DisposableStore;

	setup(() => {
		disposables = new DisposableStore();
	});

	teardown(() => {
		disposables.dispose();
	});

	ensureNoDisposablesAreLeakedInTestSuite();

	test('constructor sets providerName and conversationId', () => {
		const input = disposables.add(new ForgeChatInput('anthropic', 'conv-001'));
		assert.strictEqual(input.providerName, 'anthropic');
		assert.strictEqual(input.conversationId, 'conv-001');
	});

	test('typeId and editorId return the static ID', () => {
		const input = disposables.add(new ForgeChatInput('openai', 'conv-002'));
		assert.strictEqual(input.typeId, ForgeChatInput.ID);
		assert.strictEqual(input.editorId, ForgeChatInput.ID);
	});

	test('resource URI uses forge-chat scheme and conversationId as path', () => {
		const input = disposables.add(new ForgeChatInput('anthropic', 'conv-003'));
		assert.strictEqual(input.resource.scheme, 'forge-chat');
		assert.strictEqual(input.resource.path, '/conv-003');
	});

	test('getName includes provider name', () => {
		const input = disposables.add(new ForgeChatInput('anthropic', 'conv-004'));
		assert.strictEqual(input.getName(), 'Chat — anthropic');
	});

	test('matches returns true for same conversationId', () => {
		const input1 = disposables.add(new ForgeChatInput('anthropic', 'conv-005'));
		const input2 = disposables.add(new ForgeChatInput('anthropic', 'conv-005'));
		assert.strictEqual(input1.matches(input2), true);
	});

	test('matches returns true for same conversationId with different providers', () => {
		const input1 = disposables.add(new ForgeChatInput('anthropic', 'conv-006'));
		const input2 = disposables.add(new ForgeChatInput('openai', 'conv-006'));
		assert.strictEqual(input1.matches(input2), true);
	});

	test('matches returns false for different conversationId', () => {
		const input1 = disposables.add(new ForgeChatInput('anthropic', 'conv-007'));
		const input2 = disposables.add(new ForgeChatInput('anthropic', 'conv-008'));
		assert.strictEqual(input1.matches(input2), false);
	});
});

suite('ForgeChatInput serialization data contract', () => {

	let disposables: DisposableStore;

	setup(() => {
		disposables = new DisposableStore();
	});

	teardown(() => {
		disposables.dispose();
	});

	ensureNoDisposablesAreLeakedInTestSuite();

	test('roundtrip through JSON produces equivalent input', () => {
		const original = disposables.add(new ForgeChatInput('anthropic', 'conv-100'));

		// Serialize using the same shape the ForgeChatInputSerializer uses
		const serialized = JSON.stringify({
			providerName: original.providerName,
			conversationId: original.conversationId,
		});

		const data = JSON.parse(serialized) as { providerName: string; conversationId: string };
		const restored = disposables.add(new ForgeChatInput(data.providerName, data.conversationId));

		assert.strictEqual(restored.providerName, original.providerName);
		assert.strictEqual(restored.conversationId, original.conversationId);
		assert.strictEqual(original.matches(restored), true);
	});

	test('serialized JSON contains exactly providerName and conversationId', () => {
		const input = disposables.add(new ForgeChatInput('openai', 'conv-200'));

		const serialized = JSON.stringify({
			providerName: input.providerName,
			conversationId: input.conversationId,
		});

		const parsed = JSON.parse(serialized);
		const keys = Object.keys(parsed).sort();
		assert.deepStrictEqual(keys, ['conversationId', 'providerName']);
		assert.strictEqual(parsed.providerName, 'openai');
		assert.strictEqual(parsed.conversationId, 'conv-200');
	});

	test('deserialization of malformed JSON returns undefined (mirrors serializer behavior)', () => {
		const malformed = '{not valid json';
		let result: ForgeChatInput | undefined;
		try {
			const data = JSON.parse(malformed) as { providerName: string; conversationId: string };
			result = disposables.add(new ForgeChatInput(data.providerName, data.conversationId));
		} catch {
			result = undefined;
		}
		assert.strictEqual(result, undefined);
	});
});
