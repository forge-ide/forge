/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import assert from 'assert';
import { ForgeChatInput } from '../../../../../browser/parts/editor/forgeChat/forgeChatInput.js';
import { DisposableStore } from '../../../../../../base/common/lifecycle.js';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../../base/test/common/utils.js';
import { EditorInputCapabilities } from '../../../../../common/editor.js';
import { EditorInput } from '../../../../../common/editor/editorInput.js';
import { URI } from '../../../../../../base/common/uri.js';

class TestEditorInput extends EditorInput {
	static readonly ID = 'test.editorInput';

	get typeId(): string { return TestEditorInput.ID; }
	get resource(): URI | undefined { return undefined; }
}

suite('ForgeChatInput', () => {

	let disposables: DisposableStore;

	setup(() => {
		disposables = new DisposableStore();
	});

	teardown(() => {
		disposables.dispose();
	});

	ensureNoDisposablesAreLeakedInTestSuite();

	test('typeId returns ForgeChatInput.ID', () => {
		const input = disposables.add(new ForgeChatInput('Anthropic', 'conv-1'));
		assert.strictEqual(input.typeId, ForgeChatInput.ID);
	});

	test('editorId returns ForgeChatInput.ID', () => {
		const input = disposables.add(new ForgeChatInput('Anthropic', 'conv-1'));
		assert.strictEqual(input.editorId, ForgeChatInput.ID);
	});

	test('getName() includes the provider name', () => {
		const input = disposables.add(new ForgeChatInput('Anthropic', 'conv-1'));
		const name = input.getName();
		assert.ok(name.includes('Anthropic'), `Expected name "${name}" to include "Anthropic"`);
	});

	test('resource has the forge-chat scheme and includes the conversationId', () => {
		const input = disposables.add(new ForgeChatInput('Anthropic', 'conv-42'));
		assert.strictEqual(input.resource.scheme, 'forge-chat');
		assert.ok(input.resource.path.includes('conv-42'), `Expected path "${input.resource.path}" to include "conv-42"`);
	});

	test('capabilities includes Readonly', () => {
		const input = disposables.add(new ForgeChatInput('Anthropic', 'conv-1'));
		assert.ok(input.capabilities & EditorInputCapabilities.Readonly, 'Expected Readonly capability');
	});

	test('matches() returns true for same conversationId', () => {
		const input1 = disposables.add(new ForgeChatInput('Anthropic', 'conv-1'));
		const input2 = disposables.add(new ForgeChatInput('Anthropic', 'conv-1'));
		assert.strictEqual(input1.matches(input2), true);
	});

	test('matches() returns false for different conversationId', () => {
		const input1 = disposables.add(new ForgeChatInput('Anthropic', 'conv-1'));
		const input2 = disposables.add(new ForgeChatInput('Anthropic', 'conv-2'));
		assert.strictEqual(input1.matches(input2), false);
	});

	test('matches() returns false for a different EditorInput type', () => {
		const input = disposables.add(new ForgeChatInput('Anthropic', 'conv-1'));
		const other = disposables.add(new TestEditorInput());
		assert.strictEqual(input.matches(other), false);
	});
});
