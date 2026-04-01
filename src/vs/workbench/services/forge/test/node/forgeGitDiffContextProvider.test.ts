/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import assert from 'assert';
import { CancellationTokenSource } from '../../../../../base/common/cancellation.js';
import { DisposableStore } from '../../../../../base/common/lifecycle.js';
import { URI } from '../../../../../base/common/uri.js';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import { NullLogService } from '../../../../../platform/log/common/log.js';
import { ForgeContextType } from '../../common/forgeContextTypes.js';
import { ForgeGitDiffContextProvider } from '../../node/contextProviders/forgeGitDiffContextProvider.js';

// ---------------------------------------------------------------------------
// Testable subclass — overrides protected execGitDiff to avoid child_process
// ---------------------------------------------------------------------------

class TestableGitDiffProvider extends ForgeGitDiffContextProvider {

	private _diffOutput: string = '';
	private _shouldFail: boolean = false;
	private _failError: Error = new Error('git not found');

	constructor() {
		super(new NullLogService());
	}

	setDiffOutput(output: string): void {
		this._diffOutput = output;
		this._shouldFail = false;
	}

	setFailure(error?: Error): void {
		this._shouldFail = true;
		if (error) {
			this._failError = error;
		}
	}

	protected override async execGitDiff(_cwd: string): Promise<string> {
		if (this._shouldFail) {
			throw this._failError;
		}
		return this._diffOutput;
	}
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

suite('ForgeGitDiffContextProvider', () => {

	let disposables: DisposableStore;
	let provider: TestableGitDiffProvider;
	const workspaceRoot = URI.file('/test-workspace');

	setup(() => {
		disposables = new DisposableStore();
		provider = disposables.add(new TestableGitDiffProvider());
	});

	teardown(() => {
		disposables.dispose();
	});

	ensureNoDisposablesAreLeakedInTestSuite();

	// -------------------------------------------------------------------
	// Basic resolution
	// -------------------------------------------------------------------

	suite('resolveGitDiff', () => {

		test('returns ForgeContextItem with type GitDiff', async () => {
			provider.setDiffOutput('diff --git a/file.ts b/file.ts\n-old\n+new');

			const item = await provider.resolveGitDiff(workspaceRoot);

			assert.strictEqual(item.type, ForgeContextType.GitDiff);
		});

		test('returns label "Git Diff (HEAD)" for normal diff', async () => {
			provider.setDiffOutput('diff --git a/file.ts b/file.ts\n-old\n+new');

			const item = await provider.resolveGitDiff(workspaceRoot);

			assert.strictEqual(item.label, 'Git Diff (HEAD)');
		});

		test('content matches git output when under maxChars', async () => {
			const diffText = 'diff --git a/foo.ts b/foo.ts\n--- a/foo.ts\n+++ b/foo.ts\n@@ -1 +1 @@\n-old\n+new';
			provider.setDiffOutput(diffText);

			const item = await provider.resolveGitDiff(workspaceRoot);

			assert.strictEqual(item.content, diffText);
		});
	});

	// -------------------------------------------------------------------
	// Empty / no changes
	// -------------------------------------------------------------------

	suite('no changes', () => {

		test('empty stdout returns "No changes detected."', async () => {
			provider.setDiffOutput('');

			const item = await provider.resolveGitDiff(workspaceRoot);

			assert.strictEqual(item.content, 'No changes detected.');
			assert.strictEqual(item.label, 'Git Diff (HEAD)');
		});

		test('whitespace-only stdout returns "No changes detected."', async () => {
			provider.setDiffOutput('   \n\t\n  ');

			const item = await provider.resolveGitDiff(workspaceRoot);

			assert.strictEqual(item.content, 'No changes detected.');
		});
	});

	// -------------------------------------------------------------------
	// Truncation
	// -------------------------------------------------------------------

	suite('truncation', () => {

		test('truncates large diffs from the middle', async () => {
			const largeDiff = 'A'.repeat(2000);
			provider.setDiffOutput(largeDiff);

			const item = await provider.resolveGitDiff(workspaceRoot, 1000);

			assert.ok(item.content.includes('[...truncated'));
			assert.ok(item.content.includes('characters from middle...]'));
		});

		test('truncation preserves head and tail of diff', async () => {
			// Build a diff with recognizable head and tail
			const head = 'HEAD_MARKER' + 'x'.repeat(989); // 1000 chars
			const tail = 'y'.repeat(989) + 'TAIL_MARKER'; // 1000 chars
			const largeDiff = head + tail;
			provider.setDiffOutput(largeDiff);

			const item = await provider.resolveGitDiff(workspaceRoot, 1000);

			assert.ok(item.content.startsWith('HEAD_MARKER'), 'should preserve the head of the diff');
			assert.ok(item.content.endsWith('TAIL_MARKER'), 'should preserve the tail of the diff');
		});

		test('truncation message includes correct character count', async () => {
			const largeDiff = 'z'.repeat(3000);
			provider.setDiffOutput(largeDiff);

			const item = await provider.resolveGitDiff(workspaceRoot, 1000);

			// Truncated count = total - maxChars = 3000 - 1000 = 2000
			assert.ok(item.content.includes('truncated 2000 characters'), `expected "truncated 2000 characters" in: ${item.content.substring(0, 200)}`);
		});

		test('diff exactly at maxChars is not truncated', async () => {
			const exactDiff = 'q'.repeat(500);
			provider.setDiffOutput(exactDiff);

			const item = await provider.resolveGitDiff(workspaceRoot, 500);

			assert.strictEqual(item.content, exactDiff);
			assert.ok(!item.content.includes('[...truncated'));
		});

		test('diff one character over maxChars is truncated', async () => {
			const overByOne = 'r'.repeat(501);
			provider.setDiffOutput(overByOne);

			const item = await provider.resolveGitDiff(workspaceRoot, 500);

			assert.ok(item.content.includes('[...truncated'));
		});
	});

	// -------------------------------------------------------------------
	// Error handling
	// -------------------------------------------------------------------

	suite('error handling', () => {

		test('returns error item on process failure (does not throw)', async () => {
			provider.setFailure(new Error('git: command not found'));

			const item = await provider.resolveGitDiff(workspaceRoot);

			assert.strictEqual(item.type, ForgeContextType.GitDiff);
			assert.strictEqual(item.label, 'Git Diff (error)');
			assert.ok(item.content.includes('Git diff unavailable'));
		});

		test('error item content mentions the failure reason', async () => {
			provider.setFailure();

			const item = await provider.resolveGitDiff(workspaceRoot);

			assert.strictEqual(item.content, 'Git diff unavailable: failed to run git diff.');
		});
	});

	// -------------------------------------------------------------------
	// Token estimation
	// -------------------------------------------------------------------

	suite('token estimation', () => {

		test('token estimate is content.length / 4 rounded up', async () => {
			const diffText = 'x'.repeat(400);
			provider.setDiffOutput(diffText);

			const item = await provider.resolveGitDiff(workspaceRoot);

			assert.strictEqual(item.tokenEstimate, 100);
		});

		test('odd-length content rounds up', async () => {
			const diffText = 'x'.repeat(401);
			provider.setDiffOutput(diffText);

			const item = await provider.resolveGitDiff(workspaceRoot);

			assert.strictEqual(item.tokenEstimate, Math.ceil(401 / 4));
			assert.strictEqual(item.tokenEstimate, 101);
		});

		test('empty diff "No changes detected." has appropriate token estimate', async () => {
			provider.setDiffOutput('');

			const item = await provider.resolveGitDiff(workspaceRoot);

			// "No changes detected." is 21 chars → ceil(21/4) = 6
			assert.strictEqual(item.tokenEstimate, Math.ceil('No changes detected.'.length / 4));
		});

		test('error item has appropriate token estimate', async () => {
			provider.setFailure();

			const item = await provider.resolveGitDiff(workspaceRoot);

			const expectedContent = 'Git diff unavailable: failed to run git diff.';
			assert.strictEqual(item.tokenEstimate, Math.ceil(expectedContent.length / 4));
		});
	});

	// -------------------------------------------------------------------
	// Cancellation
	// -------------------------------------------------------------------

	suite('cancellation', () => {

		test('returns empty item when token is cancelled before execution', async () => {
			provider.setDiffOutput('diff content that should not appear');
			const cts = disposables.add(new CancellationTokenSource());
			cts.cancel();

			const item = await provider.resolveGitDiff(workspaceRoot, 32000, cts.token);

			assert.strictEqual(item.type, ForgeContextType.GitDiff);
			assert.strictEqual(item.label, 'Git Diff (empty)');
			assert.strictEqual(item.content, '');
		});

		test('empty item from cancellation has zero token estimate', async () => {
			const cts = disposables.add(new CancellationTokenSource());
			cts.cancel();

			const item = await provider.resolveGitDiff(workspaceRoot, 32000, cts.token);

			assert.strictEqual(item.tokenEstimate, 0);
		});
	});

	// -------------------------------------------------------------------
	// Default maxChars
	// -------------------------------------------------------------------

	suite('default maxChars', () => {

		test('default maxChars allows up to 32000 characters without truncation', async () => {
			const diff = 'd'.repeat(32000);
			provider.setDiffOutput(diff);

			const item = await provider.resolveGitDiff(workspaceRoot);

			assert.strictEqual(item.content, diff);
			assert.ok(!item.content.includes('[...truncated'));
		});

		test('default maxChars truncates content over 32000 characters', async () => {
			const diff = 'e'.repeat(32001);
			provider.setDiffOutput(diff);

			const item = await provider.resolveGitDiff(workspaceRoot);

			assert.ok(item.content.includes('[...truncated'));
		});
	});
});
