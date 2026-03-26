/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import gulp from 'gulp';
import * as util from './lib/util.ts';
import * as date from './lib/date.ts';
import * as task from './lib/task.ts';
import * as compilation from './lib/compilation.ts';

// Compile src/ to out-build/ with nls and inline sources in sourcemaps
export const compileBuildTask = task.define('compile-build', task.series(
	compilation.copyCodiconsTask,
	util.rimraf('out-build'),
	date.writeISODate('out-build'),
	compilation.compileApiProposalNamesTask,
	compilation.compileTask('src', 'out-build', true)
));
gulp.task(compileBuildTask);
