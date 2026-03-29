/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import assert from 'assert';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../base/test/common/utils.js';
import { IAIProviderService } from '../../common/aiProviderService.js';
import { ServiceIdentifier } from '../../../instantiation/common/instantiation.js';

suite('IAIProviderService decorator token', () => {

	ensureNoDisposablesAreLeakedInTestSuite();

	test('IAIProviderService is a non-null service identifier', () => {
		// createDecorator returns a ServiceIdentifier which is a callable function.
		// A falsy value here would mean the module failed to initialise.
		assert.ok(IAIProviderService);
	});

	test('IAIProviderService is a function (usable as a DI decorator)', () => {
		// ServiceIdentifier<T> is defined as a callable — verify the runtime shape.
		assert.strictEqual(typeof IAIProviderService, 'function');
	});

	test('IAIProviderService exposes a toString that returns the service id', () => {
		// VS Code's createDecorator attaches toString() returning the serviceId string.
		assert.strictEqual(IAIProviderService.toString(), 'aiProviderService');
	});

	test('IAIProviderService is assignable to ServiceIdentifier<IAIProviderService>', () => {
		// TypeScript compile-time check: the decorator token must be typed as
		// ServiceIdentifier<IAIProviderService>.  If the types were wrong this
		// assignment would produce a compile error.
		const token: ServiceIdentifier<IAIProviderService> = IAIProviderService;
		assert.ok(token);
	});
});
