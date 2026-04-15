import * as assert from 'assert';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import {
	createProviderHeader,
	createChatRow,
	createExpandedChatRow,
	createProviderCard,
	createModelRow,
} from '../../browser/forgeChatsViewHelpers.js';
import { ForgeChatEntry } from '../../../../services/forge/common/forgeChatService.js';
import { URI } from '../../../../../base/common/uri.js';
import { ForgeProviderConfig } from '../../../../services/forge/common/forgeConfigTypes.js';

function makeChat(overrides: Partial<ForgeChatEntry> = {}): ForgeChatEntry {
	return {
		resource: URI.parse('forge-session://test-1'),
		providerName: 'anthropic',
		label: 'Test Chat',
		currentModel: 'claude-opus-4',
		messageCount: 5,
		lastActiveAt: Date.now(),
		lastMessageSnippet: 'Hello world',
		...overrides,
	};
}

function makeProvider(overrides: Partial<ForgeProviderConfig> = {}): ForgeProviderConfig {
	return {
		name: 'anthropic',
		models: [{ id: 'claude-opus-4' }, { id: 'claude-3-5-sonnet-20241022' }],
		...overrides,
	};
}

suite('forgeChatsViewHelpers', () => {
	ensureNoDisposablesAreLeakedInTestSuite();

	suite('createProviderHeader', () => {
		test('renders provider name', () => {
			const el = createProviderHeader('anthropic');
			assert.ok(el.textContent?.includes('anthropic'));
		});

		test('has forge-provider-header class', () => {
			const el = createProviderHeader('openai');
			assert.ok(el.classList.contains('forge-provider-header'));
		});
	});

	suite('createChatRow', () => {
		test('renders chat label', () => {
			const el = createChatRow(makeChat({ label: 'My Chat' }));
			assert.ok(el.textContent?.includes('My Chat'));
		});

		test('renders model badge', () => {
			const el = createChatRow(makeChat({ currentModel: 'claude-opus-4' }));
			assert.ok(el.textContent?.includes('claude-opus-4'));
		});

		test('has forge-chat-row class', () => {
			const el = createChatRow(makeChat());
			assert.ok(el.classList.contains('forge-chat-row'));
		});

		test('sets data-resource attribute', () => {
			const resource = URI.parse('forge-session://abc');
			const el = createChatRow(makeChat({ resource }));
			assert.strictEqual(el.dataset['resource'], resource.toString());
		});

		test('omits badge when currentModel is absent', () => {
			const el = createChatRow(makeChat({ currentModel: '' }));
			assert.strictEqual(el.querySelector('.forge-chat-row__badge'), null);
		});
	});

	suite('createProviderCard', () => {
		test('renders provider name in header', () => {
			const el = createProviderCard(makeProvider({ name: 'openai' }), () => { });
			assert.ok(el.textContent?.includes('openai'));
		});

		test('renders each model', () => {
			const provider = makeProvider({ models: [{ id: 'gpt-4o' }, { id: 'gpt-4o-mini' }] });
			const el = createProviderCard(provider, () => { });
			assert.ok(el.textContent?.includes('gpt-4o'));
			assert.ok(el.textContent?.includes('gpt-4o-mini'));
		});

		test('has forge-provider-card class', () => {
			const el = createProviderCard(makeProvider(), () => { });
			assert.ok(el.classList.contains('forge-provider-card'));
		});

		test('has forge-provider-card--unconfigured class when isConfigured=false', () => {
			const el = createProviderCard(makeProvider(), () => { }, false);
			assert.ok(el.classList.contains('forge-provider-card--unconfigured'));
		});

		test('shows "Not configured" label when isConfigured=false', () => {
			const el = createProviderCard(makeProvider(), () => { }, false);
			assert.ok(el.textContent?.includes('Not configured'));
		});

		test('calls onNewChat with provider name and model id when model row clicked', () => {
			const provider = makeProvider({ name: 'anthropic', models: [{ id: 'claude-opus-4' }] });
			let captured: { providerName: string; modelId?: string } | undefined;
			const el = createProviderCard(provider, (p, m) => { captured = { providerName: p, modelId: m }; });
			// Find model row and click it
			const modelRow = el.querySelector('.forge-model-row') as HTMLElement;
			modelRow.click();
			assert.strictEqual(captured?.providerName, 'anthropic');
			assert.strictEqual(captured?.modelId, 'claude-opus-4');
		});

		test('header "+ New Chat" button calls onNewChat with provider name and first model id', () => {
			const provider = makeProvider({ name: 'anthropic', models: [{ id: 'claude-opus-4' }, { id: 'claude-3-5-sonnet-20241022' }] });
			let captured: { providerName: string; modelId?: string } | undefined;
			const el = createProviderCard(provider, (p, m) => { captured = { providerName: p, modelId: m }; });
			const btn = el.querySelector('.forge-new-chat-btn') as HTMLElement;
			btn.click();
			assert.strictEqual(captured?.providerName, 'anthropic');
			assert.strictEqual(captured?.modelId, 'claude-opus-4');
		});
	});

	suite('createExpandedChatRow', () => {
		test('has forge-chat-row--expanded class', () => {
			const el = createExpandedChatRow(makeChat(), () => { }, () => { }, () => { });
			assert.ok(el.classList.contains('forge-chat-row--expanded'));
		});

		test('clicking Open button calls onOpen', () => {
			let called = false;
			const el = createExpandedChatRow(makeChat(), () => { called = true; }, () => { }, () => { });
			const btn = Array.from(el.querySelectorAll('.forge-chat-action')).find(b => b.textContent === 'Open') as HTMLElement;
			btn.click();
			assert.strictEqual(called, true);
		});

		test('clicking Rename button calls onRename', () => {
			let called = false;
			const el = createExpandedChatRow(makeChat(), () => { }, () => { called = true; }, () => { });
			const btn = Array.from(el.querySelectorAll('.forge-chat-action')).find(b => b.textContent === 'Rename') as HTMLElement;
			btn.click();
			assert.strictEqual(called, true);
		});

		test('clicking Delete button calls onDelete', () => {
			let called = false;
			const el = createExpandedChatRow(makeChat(), () => { }, () => { }, () => { called = true; });
			const btn = Array.from(el.querySelectorAll('.forge-chat-action')).find(b => b.textContent === 'Delete') as HTMLElement;
			btn.click();
			assert.strictEqual(called, true);
		});

		test('Delete button has danger class', () => {
			const el = createExpandedChatRow(makeChat(), () => { }, () => { }, () => { });
			const btn = Array.from(el.querySelectorAll('.forge-chat-action')).find(b => b.textContent === 'Delete') as HTMLElement;
			assert.ok(btn.classList.contains('forge-chat-row__action--danger'));
		});

		test('omits badge when currentModel is absent', () => {
			const el = createExpandedChatRow(makeChat({ currentModel: '' }), () => { }, () => { }, () => { });
			assert.strictEqual(el.querySelector('.forge-chat-row__badge'), null);
		});

		test('omits snippet when lastMessageSnippet is absent', () => {
			const el = createExpandedChatRow(makeChat({ lastMessageSnippet: '' }), () => { }, () => { }, () => { });
			assert.strictEqual(el.querySelector('.forge-chat-snippet'), null);
		});
	});

	suite('createModelRow', () => {
		test('renders model id', () => {
			const el = createModelRow({ id: 'claude-opus-4' }, 'anthropic', () => { });
			assert.ok(el.textContent?.includes('claude-opus-4'));
		});

		test('has forge-model-row class', () => {
			const el = createModelRow({ id: 'gpt-4o' }, 'openai', () => { });
			assert.ok(el.classList.contains('forge-model-row'));
		});

		test('click calls onNewChat with provider name and model id', () => {
			let captured: { providerName: string; modelId?: string } | undefined;
			const el = createModelRow({ id: 'claude-opus-4' }, 'anthropic', (p, m) => { captured = { providerName: p, modelId: m }; });
			el.click();
			assert.strictEqual(captured?.providerName, 'anthropic');
			assert.strictEqual(captured?.modelId, 'claude-opus-4');
		});
	});
});
