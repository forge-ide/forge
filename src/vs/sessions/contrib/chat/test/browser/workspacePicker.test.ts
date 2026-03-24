/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import assert from 'assert';

import { URI } from '../../../../../base/common/uri.js';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import { mock } from '../../../../../base/test/common/mock.js';
import { TestInstantiationService } from '../../../../../platform/instantiation/test/common/instantiationServiceMock.js';
import { InMemoryStorageService, IStorageService } from '../../../../../platform/storage/common/storage.js';
import { IActionWidgetService } from '../../../../../platform/actionWidget/browser/actionWidget.js';
import { ICommandService } from '../../../../../platform/commands/common/commands.js';
import { IFileDialogService } from '../../../../../platform/dialogs/common/dialogs.js';
import { IUriIdentityService } from '../../../../../platform/uriIdentity/common/uriIdentity.js';
import { ExtUri } from '../../../../../base/common/resources.js';
import { WorkspacePicker } from '../../browser/workspacePicker.js';
import { SessionWorkspace, GITHUB_REMOTE_FILE_SCHEME } from '../../../sessions/common/sessionWorkspace.js';

suite('WorkspacePicker', () => {

	const ds = ensureNoDisposablesAreLeakedInTestSuite();
	let instantiationService: TestInstantiationService;

	setup(() => {
		instantiationService = ds.add(new TestInstantiationService());

		instantiationService.stub(IStorageService, ds.add(new InMemoryStorageService()));
		instantiationService.stub(IActionWidgetService, new class extends mock<IActionWidgetService>() {
			override get isVisible() { return false; }
		});
		instantiationService.stub(IFileDialogService, new class extends mock<IFileDialogService>() { });
		instantiationService.stub(ICommandService, new class extends mock<ICommandService>() { });
		instantiationService.stub(IUriIdentityService, new class extends mock<IUriIdentityService>() {
			override readonly extUri = new ExtUri(uri => false);
		});
	});

	function createPicker(): WorkspacePicker {
		return ds.add(instantiationService.createInstance(WorkspacePicker));
	}

	test('setSelectedProject with local folder', () => {
		const picker = createPicker();
		const folder = new SessionWorkspace(URI.file('/home/user/project'));

		picker.setSelectedProject(folder);

		assert.ok(picker.selectedProject);
		assert.strictEqual(picker.selectedProject.isFolder, true);
		assert.strictEqual(picker.selectedProject.uri.path, '/home/user/project');
	});

	test('setSelectedProject with GitHub repo URI', () => {
		const picker = createPicker();
		const repoUri = URI.from({ scheme: GITHUB_REMOTE_FILE_SCHEME, authority: 'github', path: '/owner/repo/HEAD' });
		const project = new SessionWorkspace(repoUri);

		picker.setSelectedProject(project);

		assert.ok(picker.selectedProject);
		assert.strictEqual(picker.selectedProject.isRepo, true);
	});

	test('onDidSelectProject fires when project is selected', () => {
		const picker = createPicker();
		const project = new SessionWorkspace(URI.file('/some/project'));

		let fired: SessionWorkspace | undefined;
		ds.add(picker.onDidSelectProject(p => { fired = p; }));

		picker.setSelectedProject(project, true);

		assert.ok(fired);
		assert.strictEqual(fired.isFolder, true);
		assert.strictEqual(fired.uri.path, '/some/project');
	});

	test('onDidSelectProject does not fire when fireEvent is false', () => {
		const picker = createPicker();
		const project = new SessionWorkspace(URI.file('/some/folder'));

		let fired = false;
		ds.add(picker.onDidSelectProject(() => { fired = true; }));

		picker.setSelectedProject(project, false);

		assert.strictEqual(fired, false);
		assert.ok(picker.selectedProject);
	});

	test('clearSelection clears the selected project', () => {
		const picker = createPicker();
		picker.setSelectedProject(new SessionWorkspace(URI.file('/folder')), false);

		assert.ok(picker.selectedProject);

		picker.clearSelection();

		assert.strictEqual(picker.selectedProject, undefined);
	});

	test('removeFromRecents clears selection if it matches', () => {
		const picker = createPicker();
		const uri = URI.file('/folder');
		picker.setSelectedProject(new SessionWorkspace(uri), false);

		picker.removeFromRecents(uri);

		assert.strictEqual(picker.selectedProject, undefined);
	});

	test('removeFromRecents preserves selection if it does not match', () => {
		const picker = createPicker();
		const selectedUri = URI.file('/selected');
		picker.setSelectedProject(new SessionWorkspace(selectedUri), false);

		picker.removeFromRecents(URI.file('/other'));

		assert.ok(picker.selectedProject);
		assert.strictEqual(picker.selectedProject.uri.path, '/selected');
	});

});
