/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import path from 'path';
import { getVersion } from './lib/getVersion.ts';
import * as task from './lib/task.ts';
import * as cp from 'child_process';
import { createReporter } from './lib/reporter.ts';

const root = path.dirname(import.meta.dirname);
const sha1 = getVersion(root);

//#region monaco type checking

function createTscCompileTask(watch: boolean) {
	return () => {
		return new Promise((resolve, reject) => {
			const args = ['./node_modules/.bin/tsc', '-p', './src/tsconfig.monaco.json', '--noEmit'];
			if (watch) {
				args.push('-w');
			}
			const child = cp.spawn(`node`, args, {
				cwd: path.join(import.meta.dirname, '..'),
				// stdio: [null, 'pipe', 'inherit']
			});
			const errors: string[] = [];
			const reporter = createReporter('monaco');

			let report: NodeJS.ReadWriteStream | undefined;
			const magic = /[\u001b\u009b][[()#;?]*(?:[0-9]{1,4}(?:;[0-9]{0,4})*)?[0-9A-ORZcf-nqry=><]/g; // https://stackoverflow.com/questions/25245716/remove-all-ansi-colors-styles-from-strings

			child.stdout.on('data', data => {
				let str = String(data);
				str = str.replace(magic, '').trim();
				if (str.indexOf('Starting compilation') >= 0 || str.indexOf('File change detected') >= 0) {
					errors.length = 0;
					report = reporter.end(false);

				} else if (str.indexOf('Compilation complete') >= 0) {
					// @ts-ignore
					report.end();

				} else if (str) {
					const match = /(.*\(\d+,\d+\): )(.*: )(.*)/.exec(str);
					if (match) {
						// trying to massage the message so that it matches the gulp-tsb error messages
						// e.g. src/vs/base/common/strings.ts(663,5): error TS2322: Type '1234' is not assignable to type 'string'.
						const fullpath = path.join(root, match[1]);
						const message = match[3];
						reporter(fullpath + message);
					} else {
						reporter(str);
					}
				}
			});
			child.on('exit', resolve);
			child.on('error', reject);
		});
	};
}

export const monacoTypecheckWatchTask = task.define('monaco-typecheck-watch', createTscCompileTask(true));

export const monacoTypecheckTask = task.define('monaco-typecheck', createTscCompileTask(false));

//#endregion
