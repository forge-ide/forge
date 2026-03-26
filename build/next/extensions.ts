/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

/**
 * Transpile a single extension's TypeScript source directory to JavaScript using esbuild.
 *
 * Usage: node build/next/extensions.ts --src <srcDir> --out <outDir>
 */

import * as esbuild from 'esbuild';
import * as fs from 'fs';
import * as path from 'path';
import { promisify } from 'util';
import glob from 'glob';

const globAsync = promisify(glob);

function getArgValue(name: string): string | undefined {
	const index = process.argv.indexOf(name);
	if (index !== -1 && index + 1 < process.argv.length) {
		return process.argv[index + 1];
	}
	return undefined;
}

const srcDir = getArgValue('--src');
const outDir = getArgValue('--out');

if (!srcDir || !outDir) {
	console.error('Usage: node build/next/extensions.ts --src <srcDir> --out <outDir>');
	process.exit(1);
}

const transformOptions: esbuild.TransformOptions = {
	loader: 'ts',
	format: 'cjs',
	target: 'es2024',
	sourcemap: 'inline',
	sourcesContent: false,
	tsconfigRaw: JSON.stringify({
		compilerOptions: {
			experimentalDecorators: true,
			useDefineForClassFields: false,
		}
	}),
};

async function transpileFile(srcPath: string, destPath: string): Promise<void> {
	const source = await fs.promises.readFile(srcPath, 'utf-8');
	const result = await esbuild.transform(source, {
		...transformOptions,
		sourcefile: srcPath,
	});
	await fs.promises.mkdir(path.dirname(destPath), { recursive: true });
	await fs.promises.writeFile(destPath, result.code);
}

async function main(): Promise<void> {
	const files = await globAsync('**/*.ts', {
		cwd: srcDir,
		ignore: ['**/*.d.ts'],
	});

	await Promise.all(files.map(file => {
		const srcPath = path.join(srcDir!, file);
		const destPath = path.join(outDir!, file.replace(/\.ts$/, '.js'));
		return transpileFile(srcPath, destPath);
	}));
}

main().catch(err => {
	console.error(err);
	process.exit(1);
});
