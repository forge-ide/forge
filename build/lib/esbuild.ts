/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import * as path from 'path';
import * as cp from 'child_process';

const root = path.dirname(path.dirname(import.meta.dirname));

/**
 * Transpile TypeScript source files to JavaScript via esbuild (one file per file, no bundling).
 *
 * @param outDir Output directory for transpiled files.
 * @param excludeTests When true, test files are excluded from transpilation.
 */
export function runEsbuildTranspile(outDir: string, excludeTests: boolean): Promise<void> {
	return new Promise((resolve, reject) => {
		const scriptPath = path.join(root, 'build/next/index.ts');
		const args = [scriptPath, 'transpile', '--out', outDir];
		if (excludeTests) {
			args.push('--exclude-tests');
		}

		const proc = cp.spawn(process.execPath, args, {
			cwd: root,
			stdio: 'inherit'
		});

		proc.on('error', reject);
		proc.on('close', code => {
			if (code === 0) {
				resolve();
			} else {
				reject(new Error(`esbuild transpile failed with exit code ${code} (outDir: ${outDir})`));
			}
		});
	});
}

/**
 * Bundle via esbuild for a specific build target.
 *
 * @param outDir Output directory for the bundle.
 * @param minify Whether to minify and mangle private fields.
 * @param nls Whether to emit NLS message files alongside the bundle.
 * @param target Build target: 'desktop' | 'server' | 'server-web' | 'web'. Defaults to 'desktop'.
 * @param sourceMapBaseUrl Optional CDN base URL to rewrite source map references.
 */
export function runEsbuildBundle(
	outDir: string,
	minify: boolean,
	nls: boolean,
	target: 'desktop' | 'server' | 'server-web' | 'web' = 'desktop',
	sourceMapBaseUrl?: string
): Promise<void> {
	return new Promise((resolve, reject) => {
		const scriptPath = path.join(root, 'build/next/index.ts');
		const args = [scriptPath, 'bundle', '--out', outDir, '--target', target];
		if (minify) {
			args.push('--minify');
			args.push('--mangle-privates');
		}
		if (nls) {
			args.push('--nls');
		}
		if (sourceMapBaseUrl) {
			args.push('--source-map-base-url', sourceMapBaseUrl);
		}

		const proc = cp.spawn(process.execPath, args, {
			cwd: root,
			stdio: 'inherit'
		});

		proc.on('error', reject);
		proc.on('close', code => {
			if (code === 0) {
				resolve();
			} else {
				reject(new Error(`esbuild bundle failed with exit code ${code} (outDir: ${outDir}, minify: ${minify}, nls: ${nls}, target: ${target})`));
			}
		});
	});
}
