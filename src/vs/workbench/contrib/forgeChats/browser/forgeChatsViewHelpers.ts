/*---------------------------------------------------------------------------------------------
 * Forge - ForgeChatsView DOM helpers
 *--------------------------------------------------------------------------------------------*/

import { ForgeChatEntry } from '../../../services/forge/common/forgeChatService.js';
import { ForgeModelConfig, ForgeProviderConfig } from '../../../services/forge/common/forgeConfigTypes.js';

/**
 * Creates the sticky provider group header row in the Chats tab.
 */
export function createProviderHeader(providerName: string): HTMLElement {
	const header = document.createElement('div');
	header.className = 'forge-provider-header';

	const dot = document.createElement('span');
	dot.className = 'forge-provider-header__dot';
	header.appendChild(dot);

	const name = document.createElement('span');
	name.className = 'forge-provider-name';
	name.textContent = providerName;
	header.appendChild(name);

	const chevron = document.createElement('span');
	chevron.className = 'forge-provider-header__chevron';
	// allow-any-unicode-next-line
	chevron.textContent = '▾';
	header.appendChild(chevron);

	return header;
}

/**
 * Creates a single collapsed chat row for the Chats tab.
 */
export function createChatRow(chat: ForgeChatEntry): HTMLElement {
	const row = document.createElement('div');
	row.className = 'forge-chat-row';
	row.dataset['resource'] = chat.resource.toString();

	const icon = document.createElement('span');
	icon.className = 'forge-chat-row-icon';
	// allow-any-unicode-next-line
	icon.textContent = '›';
	row.appendChild(icon);

	const label = document.createElement('span');
	label.className = 'forge-chat-row-label';
	label.textContent = chat.label;
	row.appendChild(label);

	if (chat.currentModel) {
		const badge = document.createElement('span');
		badge.className = 'forge-chat-row__badge';
		badge.textContent = chat.currentModel;
		badge.title = chat.currentModel;
		row.appendChild(badge);
	}

	return row;
}

/**
 * Creates the expanded inline detail for a chat row.
 */
export function createExpandedChatRow(
	chat: ForgeChatEntry,
	onOpen: () => void,
	onRename: () => void,
	onDelete: () => void,
): HTMLElement {
	const row = document.createElement('div');
	row.className = 'forge-chat-row forge-chat-row--expanded';
	row.dataset['resource'] = chat.resource.toString();

	const topRow = document.createElement('div');
	topRow.className = 'forge-chat-row-top';

	const label = document.createElement('span');
	label.className = 'forge-chat-row-label';
	label.textContent = chat.label;
	topRow.appendChild(label);

	if (chat.currentModel) {
		const badge = document.createElement('span');
		badge.className = 'forge-chat-row__badge';
		badge.textContent = chat.currentModel;
		badge.title = chat.currentModel;
		topRow.appendChild(badge);
	}

	row.appendChild(topRow);

	if (chat.lastMessageSnippet) {
		const snippet = document.createElement('div');
		snippet.className = 'forge-chat-snippet';
		snippet.textContent = `"${chat.lastMessageSnippet}"`;
		row.appendChild(snippet);
	}

	const meta = document.createElement('div');
	meta.className = 'forge-chat-meta';
	const age = _relativeTime(chat.lastActiveAt);
	meta.textContent = `${age} · ${chat.messageCount} messages`;
	row.appendChild(meta);

	const actions = document.createElement('div');
	actions.className = 'forge-chat-actions';
	actions.appendChild(_actionButton('Open', false, onOpen));
	actions.appendChild(_actionButton('Rename', false, onRename));
	actions.appendChild(_actionButton('Delete', true, onDelete));
	row.appendChild(actions);

	return row;
}

/**
 * Creates a provider card for the Providers tab.
 * onNewChat(providerName, modelId) — card-level button uses provider.models[0].id.
 * isConfigured=false renders the card dimmed with "Not configured" label.
 */
export function createProviderCard(
	provider: ForgeProviderConfig,
	onNewChat: (providerName: string, modelId?: string) => void,
	isConfigured: boolean = true,
): HTMLElement {
	const card = document.createElement('div');
	card.className = isConfigured
		? 'forge-provider-card'
		: 'forge-provider-card forge-provider-card--unconfigured';

	const header = document.createElement('div');
	header.className = 'forge-provider-card-header';

	const dot = document.createElement('span');
	dot.className = 'forge-provider-dot';
	header.appendChild(dot);

	const name = document.createElement('span');
	name.className = 'forge-provider-name';
	name.textContent = provider.name;
	header.appendChild(name);

	if (isConfigured) {
		const newChatBtn = document.createElement('button');
		newChatBtn.className = 'forge-new-chat-btn';
		newChatBtn.textContent = '+ New Chat';
		newChatBtn.addEventListener('click', (e) => {
			e.stopPropagation();
			onNewChat(provider.name, provider.models[0]?.id);
		});
		header.appendChild(newChatBtn);
	} else {
		const notConfigured = document.createElement('span');
		notConfigured.className = 'forge-provider-not-configured';
		notConfigured.textContent = 'Not configured';
		header.appendChild(notConfigured);
	}

	card.appendChild(header);

	for (const model of provider.models) {
		card.appendChild(createModelRow(model, provider.name, onNewChat));
	}

	return card;
}

/**
 * Creates a single model row within a provider card.
 */
export function createModelRow(
	model: ForgeModelConfig,
	providerName: string,
	onNewChat: (providerName: string, modelId?: string) => void,
): HTMLElement {
	const row = document.createElement('div');
	row.className = 'forge-model-row';

	const modelId = document.createElement('span');
	modelId.className = 'forge-model-id';
	modelId.textContent = model.id;
	row.appendChild(modelId);

	const newChatLink = document.createElement('span');
	newChatLink.className = 'forge-model-new-chat';
	newChatLink.textContent = '+ New Chat';
	newChatLink.addEventListener('click', (e) => {
		e.stopPropagation();
		onNewChat(providerName, model.id);
	});
	row.appendChild(newChatLink);

	row.addEventListener('click', () => onNewChat(providerName, model.id));

	return row;
}

function _actionButton(label: string, isDanger: boolean, onClick: () => void): HTMLElement {
	const btn = document.createElement('span');
	btn.className = isDanger ? 'forge-chat-action forge-chat-row__action--danger' : 'forge-chat-action';
	btn.textContent = label;
	btn.addEventListener('click', (e) => {
		e.stopPropagation();
		onClick();
	});
	return btn;
}

function _relativeTime(ms: number): string {
	const diff = Date.now() - ms;
	const mins = Math.floor(diff / 60_000);
	if (mins < 1) { return 'just now'; }
	if (mins < 60) { return `${mins}m ago`; }
	const hrs = Math.floor(mins / 60);
	if (hrs < 24) { return `${hrs}h ago`; }
	return `${Math.floor(hrs / 24)}d ago`;
}
