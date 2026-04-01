/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import assert from 'assert';
import { CancellationTokenSource } from '../../../../../base/common/cancellation.js';
import { DisposableStore } from '../../../../../base/common/lifecycle.js';
import { Schemas } from '../../../../../base/common/network.js';
import { URI } from '../../../../../base/common/uri.js';
import { VSBuffer } from '../../../../../base/common/buffer.js';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import { FileService } from '../../../../../platform/files/common/fileService.js';
import { InMemoryFileSystemProvider } from '../../../../../platform/files/common/inMemoryFilesystemProvider.js';
import { NullLogService } from '../../../../../platform/log/common/log.js';
import type { IFileService } from '../../../../../platform/files/common/files.js';
import { ForgeContextType } from '../../common/forgeContextTypes.js';
import { ForgeFileContextProvider } from '../../browser/contextProviders/forgeFileContextProvider.js';

suite('ForgeFileContextProvider', () => {

	let disposables: DisposableStore;
	let fileService: IFileService;
	let fsProvider: InMemoryFileSystemProvider;

	setup(() => {
		disposables = new DisposableStore();
		fileService = disposables.add(new FileService(new NullLogService()));
		fsProvider = disposables.add(new InMemoryFileSystemProvider());
		disposables.add(fileService.registerProvider(Schemas.file, fsProvider));
	});

	teardown(() => {
		disposables.dispose();
	});

	ensureNoDisposablesAreLeakedInTestSuite();

	function createProvider(): ForgeFileContextProvider {
		const provider = new ForgeFileContextProvider(
			fileService,
			new NullLogService(),
		);
		return disposables.add(provider);
	}

	async function writeFile(path: string, content: string): Promise<URI> {
		const uri = URI.file(path);
		await fileService.writeFile(uri, VSBuffer.fromString(content));
		return uri;
	}

	// -----------------------------------------------------------------------
	// resolveFile
	// -----------------------------------------------------------------------

	suite('resolveFile', () => {

		test('returns full content for small files', async () => {
			const provider = createProvider();
			const content = 'const hello = "world";';
			const uri = await writeFile('/workspace/src/small.ts', content);

			const item = await provider.resolveFile(uri);

			assert.strictEqual(item.type, ForgeContextType.File);
			assert.strictEqual(item.label, 'small.ts');
			assert.strictEqual(item.content, content);
			assert.deepStrictEqual(item.uri, uri);
		});

		test('truncates from middle for large files', async () => {
			const provider = createProvider();
			const content = 'x'.repeat(100_000);
			const maxChars = 1000;
			const uri = await writeFile('/workspace/src/large.ts', content);

			const item = await provider.resolveFile(uri, maxChars);

			// Content should be roughly maxChars + truncation message length
			assert.ok(item.content.length < content.length, 'content should be shorter than original');
			// First half of maxChars should be present
			const halfLen = Math.floor(maxChars / 2);
			assert.strictEqual(item.content.substring(0, halfLen), 'x'.repeat(halfLen));
			// Should contain truncation marker
			assert.ok(item.content.includes('[...truncated'), 'should contain truncation marker');
			assert.ok(item.content.includes('characters from middle...]'), 'should contain truncation message');
			// Last half should be present at the end
			assert.ok(item.content.endsWith('x'.repeat(halfLen)), 'should end with last half of content');
		});

		test('truncation preserves first half and last half', async () => {
			const provider = createProvider();
			const start = 'A'.repeat(500);
			const middle = 'B'.repeat(9000);
			const end = 'C'.repeat(500);
			const content = start + middle + end;
			const uri = await writeFile('/workspace/src/sections.ts', content);

			const item = await provider.resolveFile(uri, 1000);

			// The first 500 chars of result should be all A's
			const halfLen = Math.floor(1000 / 2);
			assert.strictEqual(item.content.substring(0, halfLen), 'A'.repeat(halfLen));
			// The last 500 chars of result should be all C's
			assert.strictEqual(item.content.slice(-halfLen), 'C'.repeat(halfLen));
			// B's from the middle should not appear (they were truncated)
			assert.ok(!item.content.includes('B'), 'middle section should be truncated');
		});

		test('truncation message includes accurate character count', async () => {
			const provider = createProvider();
			const content = 'z'.repeat(10_000);
			const maxChars = 2000;
			const uri = await writeFile('/workspace/src/counted.ts', content);

			const item = await provider.resolveFile(uri, maxChars);

			const expectedTruncated = 10_000 - 2000; // 8000
			assert.ok(
				item.content.includes(`truncated ${expectedTruncated} characters`),
				`should report ${expectedTruncated} truncated characters, got: ${item.content.match(/truncated \d+ characters/)?.[0]}`
			);
		});

		test('returns correct tokenEstimate', async () => {
			const provider = createProvider();

			// 400 chars => 100 tokens
			const uri400 = await writeFile('/workspace/src/four-hundred.ts', 'a'.repeat(400));
			const item400 = await provider.resolveFile(uri400);
			assert.strictEqual(item400.tokenEstimate, 100);

			// 401 chars => ceil(401/4) = 101 tokens
			const uri401 = await writeFile('/workspace/src/four-hundred-one.ts', 'b'.repeat(401));
			const item401 = await provider.resolveFile(uri401);
			assert.strictEqual(item401.tokenEstimate, 101);
		});

		test('handles file read errors gracefully', async () => {
			const provider = createProvider();
			const uri = URI.file('/workspace/nonexistent/missing.ts');

			// Should not throw
			const item = await provider.resolveFile(uri);

			assert.strictEqual(item.type, ForgeContextType.File);
			assert.strictEqual(item.label, 'missing.ts');
			assert.ok(item.content.startsWith('[Error reading file:'), `expected error content, got: ${item.content}`);
			assert.deepStrictEqual(item.uri, uri);
		});

		test('returns correct label as file basename', async () => {
			const provider = createProvider();
			const uri = await writeFile('/workspace/src/foo/bar.ts', 'content');

			const item = await provider.resolveFile(uri);

			assert.strictEqual(item.label, 'bar.ts');
		});

		test('returns correct uri on the item', async () => {
			const provider = createProvider();
			const uri = await writeFile('/workspace/src/check-uri.ts', 'content');

			const item = await provider.resolveFile(uri);

			assert.deepStrictEqual(item.uri, uri);
		});

		test('respects custom maxChars parameter', async () => {
			const provider = createProvider();
			const content = 'y'.repeat(5000);
			const maxChars = 500;
			const uri = await writeFile('/workspace/src/custom-max.ts', content);

			const item = await provider.resolveFile(uri, maxChars);

			// The raw content portion should be maxChars (two halves of 250 each)
			const halfLen = Math.floor(maxChars / 2);
			assert.strictEqual(item.content.substring(0, halfLen), 'y'.repeat(halfLen));
			assert.ok(item.content.includes('[...truncated'), 'should be truncated');
			// Total content should be maxChars + the truncation message (with newlines)
			const truncationMessage = `\n\n[...truncated ${5000 - maxChars} characters from middle...]\n\n`;
			assert.strictEqual(item.content.length, maxChars + truncationMessage.length);
		});

		test('returns empty content when cancelled', async () => {
			const provider = createProvider();
			const uri = await writeFile('/workspace/src/cancelled.ts', 'should not appear');

			const cts = new CancellationTokenSource();
			cts.cancel();

			const item = await provider.resolveFile(uri, undefined, cts.token);

			assert.strictEqual(item.type, ForgeContextType.File);
			assert.strictEqual(item.content, '');
			assert.strictEqual(item.tokenEstimate, 0);
			assert.strictEqual(item.label, 'cancelled.ts');

			cts.dispose();
		});

		test('does not truncate when content is exactly at maxChars', async () => {
			const provider = createProvider();
			const content = 'q'.repeat(1000);
			const uri = await writeFile('/workspace/src/exact.ts', content);

			const item = await provider.resolveFile(uri, 1000);

			assert.strictEqual(item.content, content);
			assert.ok(!item.content.includes('truncated'), 'should not be truncated when exactly at limit');
		});

		test('does not truncate when content is under maxChars', async () => {
			const provider = createProvider();
			const content = 'r'.repeat(500);
			const uri = await writeFile('/workspace/src/under.ts', content);

			const item = await provider.resolveFile(uri, 1000);

			assert.strictEqual(item.content, content);
			assert.ok(!item.content.includes('truncated'), 'should not be truncated when under limit');
		});

		test('detail field contains the file path', async () => {
			const provider = createProvider();
			const uri = await writeFile('/workspace/src/detail-check.ts', 'content');

			const item = await provider.resolveFile(uri);

			assert.strictEqual(item.detail, uri.path);
		});

		test('tokenEstimate is based on final content after truncation', async () => {
			const provider = createProvider();
			const content = 'w'.repeat(10_000);
			const maxChars = 200;
			const uri = await writeFile('/workspace/src/token-trunc.ts', content);

			const item = await provider.resolveFile(uri, maxChars);

			// tokenEstimate should reflect the truncated content, not original
			const expectedEstimate = Math.ceil(item.content.length / 4);
			assert.strictEqual(item.tokenEstimate, expectedEstimate);
		});
	});
});
