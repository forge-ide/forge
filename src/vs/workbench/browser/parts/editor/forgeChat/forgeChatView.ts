/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { Disposable } from '../../../../../base/common/lifecycle.js';
import { $, append, addDisposableListener, EventType } from '../../../../../base/browser/dom.js';
import { IAIProviderService } from '../../../../../platform/ai/common/aiProviderService.js';
import { AIMessage } from '../../../../../platform/ai/common/aiProvider.js';
import { ILogService } from '../../../../../platform/log/common/log.js';
import { IForgeConfigService } from '../../../../services/forge/common/forgeConfigService.js';

const DEFAULT_MODEL = 'claude-sonnet-4-6';

export class ForgeChatView extends Disposable {
	private readonly messagesContainer: HTMLElement;
	private readonly inputArea: HTMLTextAreaElement;
	private readonly sendButton: HTMLElement;
	private readonly providerDot: HTMLElement;
	private readonly modelLabel: HTMLElement;

	private currentConversationId: string = '';
	private readonly conversations = new Map<string, AIMessage[]>();
	private messages: AIMessage[] = [];
	private providerName: string = '';
	private model: string = '';
	private isStreaming = false;

	constructor(
		parent: HTMLElement,
		@IAIProviderService private readonly aiProviderService: IAIProviderService,
		@ILogService private readonly logService: ILogService,
		@IForgeConfigService private readonly forgeConfigService: IForgeConfigService,
	) {
		super();

		// Initialize from forge.json config
		const config = this.forgeConfigService.getConfig();
		this.providerName = config.provider;
		this.model = config.model ?? DEFAULT_MODEL;

		// Root
		const root = $('.forge-chat-pane');

		// Header
		const header = $('.forge-chat-header');
		this.providerDot = append(header, $('.forge-provider-dot'));
		this.modelLabel = append(header, $('.forge-model-label'));
		this.modelLabel.textContent = this.model;

		// Messages area
		this.messagesContainer = $('.forge-chat-messages');

		// Input area
		const inputWrapper = $('.forge-chat-input-area');
		this.inputArea = append(inputWrapper, $<HTMLTextAreaElement>('textarea.forge-chat-input'));
		this.inputArea.placeholder = 'Type a message...';
		this.inputArea.rows = 1;

		this.sendButton = append(inputWrapper, $('.forge-chat-send'));
		this.sendButton.textContent = 'Send';

		// Assemble DOM in correct order: header, messages, input
		append(root, header);
		append(root, this.messagesContainer);
		append(root, inputWrapper);
		append(parent, root);

		// Wire up events
		this._register(addDisposableListener(this.sendButton, EventType.CLICK, () => {
			this.sendMessage();
		}));

		this._register(addDisposableListener(this.inputArea, EventType.KEY_DOWN, (e: KeyboardEvent) => {
			if (e.key === 'Enter' && !e.shiftKey) {
				e.preventDefault();
				this.sendMessage();
			}
		}));

		this._register(addDisposableListener(this.inputArea, 'input', () => {
			this.autoResizeInput();
		}));

		this._register(this.aiProviderService.onDidChangeProvider((name: string) => {
			this.providerName = name;
			this.updateHeader();
		}));

		this._register(this.forgeConfigService.onDidChange(config => {
			this.model = config.model ?? this.model;
			this.updateHeader();
		}));
	}

	setConversation(conversationId: string, providerName: string, model?: string): void {
		// Save current conversation's messages
		if (this.currentConversationId) {
			this.conversations.set(this.currentConversationId, this.messages);
		}

		// Switch to new conversation
		this.currentConversationId = conversationId;
		this.providerName = providerName;
		if (model) {
			this.model = model;
		}

		// Restore or start fresh
		this.messages = this.conversations.get(conversationId) ?? [];

		// Rebuild message DOM
		this.messagesContainer.textContent = '';
		for (const msg of this.messages) {
			this.appendMessageElement(msg.role, msg.content);
		}

		this.updateHeader();
	}

	private updateHeader(): void {
		this.modelLabel.textContent = this.model;
		this.providerDot.className = `forge-provider-dot provider-${this.providerName}`;
	}

	private async sendMessage(): Promise<void> {
		const text = this.inputArea.value.trim();
		if (!text || this.isStreaming) {
			return;
		}

		this.inputArea.value = '';
		this.autoResizeInput();
		this.messages.push({ role: 'user', content: text });
		this.appendMessageElement('user', text);

		const provider = this.aiProviderService.getActiveProvider();
		if (!provider) {
			this.logService.warn('[ForgeChatView] No active provider');
			const errorContent = 'No AI provider configured. Please set up a provider first.';
			this.messages.push({ role: 'assistant', content: errorContent });
			this.appendMessageElement('assistant', errorContent);
			return;
		}

		this.isStreaming = true;
		this.sendButton.classList.add('disabled');
		const { element: assistantEl, contentElement: contentEl } = this.appendMessageElement('assistant', '');
		assistantEl.classList.add('streaming');

		let fullContent = '';
		try {
			for await (const chunk of provider.stream({
				model: this.model,
				messages: this.messages,
			})) {
				fullContent += chunk.delta;
				contentEl.textContent = fullContent;
				this.scrollToBottom();
				if (chunk.done) {
					break;
				}
			}
		} catch (error) {
			this.logService.error('[ForgeChatView] Stream error', error);
			contentEl.textContent = fullContent || 'Error: Failed to get response';
			contentEl.classList.add('error');
		}

		assistantEl.classList.remove('streaming');
		this.isStreaming = false;
		this.sendButton.classList.remove('disabled');
		this.messages.push({ role: 'assistant', content: fullContent });
	}

	private appendMessageElement(role: string, content: string): { element: HTMLElement; contentElement: HTMLElement } {
		const msgEl = $(`.forge-message.${role}`);
		const contentEl = append(msgEl, $('.forge-message-content'));
		contentEl.textContent = content;

		if (role === 'assistant') {
			append(msgEl, $('.forge-streaming-cursor'));
		}

		append(this.messagesContainer, msgEl);
		this.scrollToBottom();
		return { element: msgEl, contentElement: contentEl };
	}

	private scrollToBottom(): void {
		this.messagesContainer.scrollTop = this.messagesContainer.scrollHeight;
	}

	private autoResizeInput(): void {
		this.inputArea.style.height = 'auto';
		this.inputArea.style.height = `${Math.min(this.inputArea.scrollHeight, 120)}px`;
	}

	layout(_width: number, _height: number): void {
		// Reserved for future resize handling
	}
}
