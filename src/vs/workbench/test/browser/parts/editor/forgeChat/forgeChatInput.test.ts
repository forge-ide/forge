/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import assert from 'assert';
import { ForgeChatInput } from '../../../../../browser/parts/editor/forgeChat/forgeChatInput.js';
import { DisposableStore } from '../../../../../../base/common/lifecycle.js';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../../base/test/common/utils.js';
import { EditorInputCapabilities, IEditorSerializer } from '../../../../../common/editor.js';
import { EditorInput } from '../../../../../common/editor/editorInput.js';
import { URI } from '../../../../../../base/common/uri.js';
import { PanePosition } from '../../../../../services/forge/common/forgeLayoutService.js';
import { IInstantiationService } from '../../../../../../platform/instantiation/common/instantiation.js';

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

/**
 * Serializer matching the logic in forgeChat.contribution.ts.
 * The contribution's ForgeChatInputSerializer is not exported, so we
 * replicate its contract here to test the serialization roundtrip.
 */
interface ForgeChatSerializedData {
	providerName: string;
	conversationId: string;
	panePosition?: string;
	model?: string;
}

class TestForgeChatInputSerializer implements IEditorSerializer {
	canSerialize(editor: EditorInput): boolean {
		return editor instanceof ForgeChatInput;
	}

	serialize(editor: EditorInput): string | undefined {
		if (!(editor instanceof ForgeChatInput)) {
			return undefined;
		}
		const data: ForgeChatSerializedData = {
			providerName: editor.providerName,
			conversationId: editor.conversationId,
			panePosition: editor.panePosition,
			model: editor.model,
		};
		return JSON.stringify(data);
	}

	deserialize(_instantiationService: IInstantiationService, serializedEditor: string): EditorInput | undefined {
		try {
			const data = JSON.parse(serializedEditor) as ForgeChatSerializedData;
			const input = new ForgeChatInput(data.providerName, data.conversationId);
			if (data.panePosition) {
				input.panePosition = data.panePosition as PanePosition;
			}
			if (data.model) {
				input.setModel(data.model);
			}
			return input;
		} catch {
			return undefined;
		}
	}
}

suite('ForgeChatInput — Enhanced Fields', () => {

	let disposables: DisposableStore;

	setup(() => {
		disposables = new DisposableStore();
	});

	teardown(() => {
		disposables.dispose();
	});

	ensureNoDisposablesAreLeakedInTestSuite();

	test('setPanePosition updates panePosition getter', () => {
		const input = disposables.add(new ForgeChatInput('Anthropic', 'conv-1'));

		assert.strictEqual(input.panePosition, undefined, 'panePosition should start undefined');

		input.panePosition = 'tl';
		assert.strictEqual(input.panePosition, 'tl');

		input.panePosition = 'br';
		assert.strictEqual(input.panePosition, 'br');
	});

	test('getName includes pane position when set', () => {
		const input = disposables.add(new ForgeChatInput('Anthropic', 'conv-1'));

		input.panePosition = 'tl';
		const name = input.getName();
		assert.ok(name.includes('Top Left'), `Expected name "${name}" to include "Top Left"`);
	});

	test('setModel updates model and fires onDidChangeLabel', () => {
		const input = disposables.add(new ForgeChatInput('Anthropic', 'conv-1'));

		let labelChangeCount = 0;
		disposables.add(input.onDidChangeLabel(() => { labelChangeCount++; }));

		assert.strictEqual(input.model, undefined, 'model should start undefined');

		input.setModel('gpt-4');
		assert.strictEqual(input.model, 'gpt-4');
		assert.strictEqual(labelChangeCount, 1, 'onDidChangeLabel should have fired once');
	});

	test('serialization roundtrip preserves panePosition and model', () => {
		const serializer = new TestForgeChatInputSerializer();

		const original = disposables.add(new ForgeChatInput('Anthropic', 'conv-rt'));
		original.panePosition = 'tr';
		original.setModel('claude-3');

		const json = serializer.serialize(original);
		assert.ok(json !== undefined, 'serialize should return a string');

		const restored = serializer.deserialize(undefined as unknown as IInstantiationService, json);
		assert.ok(restored instanceof ForgeChatInput, 'deserialize should return a ForgeChatInput');
		disposables.add(restored);

		assert.strictEqual(restored.providerName, 'Anthropic');
		assert.strictEqual(restored.conversationId, 'conv-rt');
		assert.strictEqual(restored.panePosition, 'tr');
		assert.strictEqual(restored.model, 'claude-3');
	});
});
