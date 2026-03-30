/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import assert from 'assert';
import { DisposableStore } from '../../../../../base/common/lifecycle.js';
import { Emitter, Event } from '../../../../../base/common/event.js';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import { Registry } from '../../../../../platform/registry/common/platform.js';
import {
	Extensions as ViewExtensions,
	IViewContainersRegistry,
	IViewDescriptor,
	IViewsRegistry,
	ViewContainerLocation,
} from '../../../../common/views.js';
import { FORGE_AI_VIEWLET_ID, FORGE_AI_WORKSPACE_VIEW_ID } from '../../../../contrib/forgeAI/common/forgeAI.js';
import { ForgeAIWorkspaceView } from '../../../../contrib/forgeAI/browser/forgeAIWorkspaceView.js';
import { ForgeConfig, IForgeConfigService } from '../../../../services/forge/common/forgeConfigService.js';
import { IViewletViewOptions } from '../../../../browser/parts/views/viewsViewlet.js';
import { ICommandService } from '../../../../../platform/commands/common/commands.js';

// Import the contribution to trigger side-effect registrations
import '../../../../contrib/forgeAI/browser/forgeAI.contribution.js';

/**
 * Test subclass that exposes the protected `renderBody` method so we can
 * verify DOM output without going through the full `render()` pipeline
 * (which requires many real services for toolbar creation, focus tracking, etc.).
 */
class TestableForgeAIWorkspaceView extends ForgeAIWorkspaceView {
	public testRenderBody(container: HTMLElement): void {
		this.renderBody(container);
	}
}

/**
 * Create a mock service proxy that returns a no-op function (which itself
 * returns a disposable) for every property access. Suitable for services
 * whose methods are not exercised in the test.
 */
function createNoopServiceProxy(): Record<string, unknown> {
	return new Proxy({}, {
		get: () => () => ({ dispose() { /* noop */ } }),
	});
}

/**
 * Build a `TestableForgeAIWorkspaceView` with the given mock overrides.
 * Services not provided are stubbed with no-op proxies.
 */
function createTestView(
	disposables: DisposableStore,
	overrides: {
		commandService?: Partial<ICommandService>;
		forgeConfigService?: Partial<IForgeConfigService>;
	} = {},
): TestableForgeAIWorkspaceView {
	const noop = createNoopServiceProxy();
	const noopEvent = Event.None;

	const defaultConfigEmitter = disposables.add(new Emitter<ForgeConfig>());
	const defaultForgeConfigService: Pick<IForgeConfigService, 'onDidChange' | 'getConfig'> = {
		onDidChange: defaultConfigEmitter.event,
		getConfig(): ForgeConfig { return { provider: 'anthropic', model: 'claude-sonnet-4-6' }; },
	};

	const commandService = (overrides.commandService ?? noop) as ICommandService;
	const forgeConfigService = (overrides.forgeConfigService ?? defaultForgeConfigService) as IForgeConfigService;

	const noopConfigService = {
		getValue: () => undefined,
		onDidChangeConfiguration: noopEvent,
	};

	const noopContextKeyService = {
		onDidChangeContext: noopEvent,
		createScoped: () => noop,
		contextMatchesRules: () => false,
		getContextKeyValue: () => undefined,
		createKey: () => ({ set() { /* noop */ }, reset() { /* noop */ }, get() { return undefined; } }),
	};

	const noopViewDescriptorService = {
		getViewLocationById: () => null,
		getViewContainerByViewId: () => null,
		getViewDescriptorById: () => null,
		onDidChangeLocation: noopEvent,
		onDidChangeContainer: noopEvent,
	};

	const noopInstantiationService = {
		createInstance: () => ({ dispose() { /* noop */ }, layout() { /* noop */ } }),
		invokeFunction: () => undefined,
	};

	const options: IViewletViewOptions = {
		id: ForgeAIWorkspaceView.ID,
		title: 'AI Workspaces',
	};

	return disposables.add(new TestableForgeAIWorkspaceView(
		options,
		commandService,
		forgeConfigService,
		noop as never, // keybindingService
		noop as never, // contextMenuService
		noopConfigService as never, // configurationService
		noopContextKeyService as never, // contextKeyService
		noopViewDescriptorService as never, // viewDescriptorService
		noopInstantiationService as never, // instantiationService
		noop as never, // openerService
		noop as never, // themeService
		noop as never, // hoverService
	));
}

suite('Forge AI Viewlet', () => {

	const disposables = new DisposableStore();

	teardown(() => {
		disposables.clear();
	});

	ensureNoDisposablesAreLeakedInTestSuite();

	// -------------------------------------------------------------------------
	// Constants
	// -------------------------------------------------------------------------

	suite('Constants', () => {

		test('FORGE_AI_VIEWLET_ID has the expected value', () => {
			assert.strictEqual(FORGE_AI_VIEWLET_ID, 'workbench.view.forgeAI');
		});

		test('FORGE_AI_WORKSPACE_VIEW_ID has the expected value', () => {
			assert.strictEqual(FORGE_AI_WORKSPACE_VIEW_ID, 'workbench.forgeAI.workspaceView');
		});
	});

	// -------------------------------------------------------------------------
	// View Container Registration
	// -------------------------------------------------------------------------

	suite('View Container Registration', () => {

		test('view container is registered in the ViewContainersRegistry', () => {
			const registry = Registry.as<IViewContainersRegistry>(ViewExtensions.ViewContainersRegistry);
			const all = registry.all;
			const found = all.find((vc: { id: string }) => vc.id === FORGE_AI_VIEWLET_ID);
			assert.ok(found, `Expected to find view container with id "${FORGE_AI_VIEWLET_ID}"`);
		});

		test('view container is registered in the Sidebar location', () => {
			const registry = Registry.as<IViewContainersRegistry>(ViewExtensions.ViewContainersRegistry);
			const sidebarContainers = registry.getViewContainers(ViewContainerLocation.Sidebar);
			const found = sidebarContainers.find((vc: { id: string }) => vc.id === FORGE_AI_VIEWLET_ID);
			assert.ok(found, `Expected view container "${FORGE_AI_VIEWLET_ID}" to be in the Sidebar`);
		});

		test('view container has the correct title', () => {
			const registry = Registry.as<IViewContainersRegistry>(ViewExtensions.ViewContainersRegistry);
			const container = registry.all.find((vc: { id: string }) => vc.id === FORGE_AI_VIEWLET_ID);
			assert.ok(container);
			assert.ok(container.title.value.includes('Forge AI'), `Expected title to include "Forge AI", got "${container.title.value}"`);
		});
	});

	// -------------------------------------------------------------------------
	// View Descriptor Registration
	// -------------------------------------------------------------------------

	suite('View Descriptor Registration', () => {

		test('workspace view is registered in the ViewsRegistry', () => {
			const viewsRegistry = Registry.as<IViewsRegistry>(ViewExtensions.ViewsRegistry);
			const view = viewsRegistry.getView(FORGE_AI_WORKSPACE_VIEW_ID);
			assert.ok(view, `Expected to find view descriptor with id "${FORGE_AI_WORKSPACE_VIEW_ID}"`);
		});

		test('workspace view is registered against the Forge AI view container', () => {
			const viewsRegistry = Registry.as<IViewsRegistry>(ViewExtensions.ViewsRegistry);
			const container = viewsRegistry.getViewContainer(FORGE_AI_WORKSPACE_VIEW_ID);
			assert.ok(container, `Expected view "${FORGE_AI_WORKSPACE_VIEW_ID}" to be in a container`);
			assert.strictEqual(container.id, FORGE_AI_VIEWLET_ID);
		});

		test('workspace view has canToggleVisibility set to false', () => {
			const viewsRegistry = Registry.as<IViewsRegistry>(ViewExtensions.ViewsRegistry);
			const view = viewsRegistry.getView(FORGE_AI_WORKSPACE_VIEW_ID) as IViewDescriptor;
			assert.ok(view);
			assert.strictEqual(view.canToggleVisibility, false);
		});

		test('workspace view has canMoveView set to true', () => {
			const viewsRegistry = Registry.as<IViewsRegistry>(ViewExtensions.ViewsRegistry);
			const view = viewsRegistry.getView(FORGE_AI_WORKSPACE_VIEW_ID) as IViewDescriptor;
			assert.ok(view);
			assert.strictEqual(view.canMoveView, true);
		});

		test('workspace view has an openCommandActionDescriptor with the container id', () => {
			const viewsRegistry = Registry.as<IViewsRegistry>(ViewExtensions.ViewsRegistry);
			const view = viewsRegistry.getView(FORGE_AI_WORKSPACE_VIEW_ID) as IViewDescriptor;
			assert.ok(view);
			assert.ok(view.openCommandActionDescriptor, 'Expected openCommandActionDescriptor to be defined');
			assert.strictEqual(view.openCommandActionDescriptor.id, FORGE_AI_VIEWLET_ID);
		});
	});

	// -------------------------------------------------------------------------
	// ForgeAIWorkspaceView
	// -------------------------------------------------------------------------

	suite('ForgeAIWorkspaceView', () => {

		test('static ID matches the constant', () => {
			assert.strictEqual(ForgeAIWorkspaceView.ID, FORGE_AI_WORKSPACE_VIEW_ID);
		});

		test('renders a New Chat button that executes forge.chat.new', () => {
			let executedCommand: string | undefined;

			const mockCommandService: Pick<ICommandService, 'executeCommand'> = {
				executeCommand(id: string): Promise<undefined> {
					executedCommand = id;
					return Promise.resolve(undefined);
				},
			};

			const onDidChangeEmitter = disposables.add(new Emitter<ForgeConfig>());
			const mockForgeConfigService: Pick<IForgeConfigService, 'onDidChange' | 'getConfig'> = {
				onDidChange: onDidChangeEmitter.event,
				getConfig(): ForgeConfig {
					return { provider: 'anthropic', model: 'claude-sonnet-4-6' };
				},
			};

			const view = createTestView(disposables, {
				commandService: mockCommandService,
				forgeConfigService: mockForgeConfigService,
			});

			const container = document.createElement('div');
			view.testRenderBody(container);

			// Find the button
			const button = container.querySelector('button.forge-ai-new-chat-button');
			assert.ok(button, 'Expected a <button> with class "forge-ai-new-chat-button"');
			assert.ok(button.textContent?.includes('NEW CHAT'), `Expected button text to include "NEW CHAT", got "${button.textContent}"`);

			// Click the button and verify the command was dispatched
			button.dispatchEvent(new MouseEvent('click', { bubbles: true }));
			assert.strictEqual(executedCommand, 'forge.chat.new', 'Expected click to execute "forge.chat.new" command');
		});

		test('displays provider info from ForgeConfigService', () => {
			const onDidChangeEmitter = disposables.add(new Emitter<ForgeConfig>());
			const mockForgeConfigService: Pick<IForgeConfigService, 'onDidChange' | 'getConfig'> = {
				onDidChange: onDidChangeEmitter.event,
				getConfig(): ForgeConfig {
					return { provider: 'openai', model: 'gpt-4o' };
				},
			};

			const view = createTestView(disposables, {
				forgeConfigService: mockForgeConfigService,
			});

			const container = document.createElement('div');
			view.testRenderBody(container);

			const providerLabel = container.querySelector('.forge-ai-provider-label');
			assert.ok(providerLabel, 'Expected a provider label element');
			assert.ok(providerLabel.textContent?.includes('openai'), `Expected provider label to include "openai", got "${providerLabel.textContent}"`);
			assert.ok(providerLabel.textContent?.includes('gpt-4o'), `Expected provider label to include "gpt-4o", got "${providerLabel.textContent}"`);
		});

		test('updates provider display when config changes', () => {
			const onDidChangeEmitter = disposables.add(new Emitter<ForgeConfig>());
			let currentConfig: ForgeConfig = { provider: 'anthropic', model: 'claude-sonnet-4-6' };
			const mockForgeConfigService: Pick<IForgeConfigService, 'onDidChange' | 'getConfig'> = {
				onDidChange: onDidChangeEmitter.event,
				getConfig(): ForgeConfig {
					return currentConfig;
				},
			};

			const view = createTestView(disposables, {
				forgeConfigService: mockForgeConfigService,
			});

			const container = document.createElement('div');
			view.testRenderBody(container);

			// Verify initial state
			const providerLabel = container.querySelector('.forge-ai-provider-label');
			assert.ok(providerLabel);
			assert.ok(providerLabel.textContent?.includes('anthropic'));

			// Simulate config change
			currentConfig = { provider: 'openai', model: 'gpt-4o' };
			onDidChangeEmitter.fire(currentConfig);

			assert.ok(providerLabel.textContent?.includes('openai'), `Expected updated label to include "openai", got "${providerLabel.textContent}"`);
			assert.ok(providerLabel.textContent?.includes('gpt-4o'), `Expected updated label to include "gpt-4o", got "${providerLabel.textContent}"`);
		});
	});
});
