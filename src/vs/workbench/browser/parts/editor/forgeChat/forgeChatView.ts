/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { Disposable, DisposableStore } from '../../../../../base/common/lifecycle.js';
import { $, append, addDisposableListener, EventType } from '../../../../../base/browser/dom.js';
import { IAIProviderService } from '../../../../../platform/ai/common/aiProviderService.js';
import { AIMessage } from '../../../../../platform/ai/common/aiProvider.js';
import { ILogService } from '../../../../../platform/log/common/log.js';
import { IForgeConfigService } from '../../../../services/forge/common/forgeConfigService.js';
import { IForgeContextService } from '../../../../services/forge/common/forgeContextService.js';
import { ForgeContextChip, ForgeContextType, formatContextItem } from '../../../../services/forge/common/forgeContextTypes.js';
import type { PanePosition } from '../../../../services/forge/common/forgeLayoutService.js';
import '../../../../services/forge/browser/media/forgeContext.css';

export class ForgeChatView extends Disposable {
	private readonly messagesContainer: HTMLElement;
	private readonly inputArea: HTMLTextAreaElement;
	private readonly sendButton: HTMLElement;
	private readonly providerDot: HTMLElement;
	private readonly modelLabel: HTMLElement;
	private readonly contextChipsContainer: HTMLElement;

	private readonly _chipListenerStore = this._register(new DisposableStore());
	private currentConversationId: string = '';
	private readonly conversations = new Map<string, AIMessage[]>();
	private messages: AIMessage[] = [];
	private providerName: string = '';
	private model: string = '';
	private isStreaming = false;
	private panePosition: PanePosition | undefined;

	onFirstMessage: (() => void) | undefined;

	constructor(
		parent: HTMLElement,
		@IAIProviderService private readonly aiProviderService: IAIProviderService,
		@ILogService private readonly logService: ILogService,
		@IForgeConfigService private readonly forgeConfigService: IForgeConfigService,
		@IForgeContextService private readonly forgeContextService: IForgeContextService,
	) {
		super();

		// Initialize from forge.json config
		const config = this.forgeConfigService.getConfig();
		this.providerName = config.defaultProvider;
		this.model = config.defaultModel ?? '';

		// Root
		const root = $('.forge-chat-pane');

		// Header
		const header = $('.forge-chat-header');
		this.providerDot = append(header, $('.forge-provider-dot'));
		this.modelLabel = append(header, $('.forge-model-label'));
		this.modelLabel.textContent = this.model || 'No model configured';
		this.providerDot.className = this.providerName && this.model
			? `forge-provider-dot provider-${this.providerName}`
			: 'forge-provider-dot';

		// Messages area
		this.messagesContainer = $('.forge-chat-messages');

		// Context chips container
		this.contextChipsContainer = $('.forge-context-chips');

		// Input area
		const inputWrapper = $('.forge-chat-input-area');
		this.inputArea = append(inputWrapper, $<HTMLTextAreaElement>('textarea.forge-chat-input'));
		this.inputArea.placeholder = 'Type a message...';
		this.inputArea.rows = 1;

		this.sendButton = append(inputWrapper, $('.forge-chat-send'));
		this.sendButton.textContent = 'Send';

		// Assemble DOM in correct order: header, messages, context chips, input
		append(root, header);
		append(root, this.messagesContainer);
		append(root, this.contextChipsContainer);
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
			this.handleAtTrigger();
		}));

		this._register(this.aiProviderService.onDidChangeProviders(() => {
			this.updateHeader();
		}));

		this._register(this.forgeConfigService.onDidChange(config => {
			// Update defaults only if this pane hasn't been explicitly configured
			if (!this.providerName || this.providerName === config.defaultProvider) {
				this.providerName = config.defaultProvider;
				this.model = config.defaultModel ?? this.model;
			}
			this.updateHeader();
		}));

		this._register(this.forgeContextService.onDidChangeContext(pos => {
			if (pos === this.panePosition || pos === undefined) {
				this.renderContextChips();
			}
		}));

		this.updateInputState();
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
		this.modelLabel.textContent = this.model || 'No model configured';
		const hasProvider = !!this.providerName;
		const hasModel = !!this.model;
		this.providerDot.className = hasProvider && hasModel
			? `forge-provider-dot provider-${this.providerName}`
			: 'forge-provider-dot';
		this.updateInputState();
	}

	private updateInputState(): void {
		const ready = !!this.providerName && !!this.model;
		this.inputArea.disabled = !ready;
		this.inputArea.placeholder = ready
			? 'Type a message...'
			: 'Configure a provider and model to start chatting';
		if (ready) {
			this.sendButton.classList.remove('disabled');
		} else {
			this.sendButton.classList.add('disabled');
		}
	}

	private async sendMessage(): Promise<void> {
		const text = this.inputArea.value.trim();
		if (!text || this.isStreaming) {
			return;
		}

		this.inputArea.value = '';
		this.autoResizeInput();
		if (this.messages.length === 0) {
			this.onFirstMessage?.();
		}
		this.messages.push({ role: 'user', content: text });
		this.appendMessageElement('user', text);

		const provider = this.providerName ? this.aiProviderService.getProvider(this.providerName) : undefined;
		if (!provider) {
			this.logService.warn(`[ForgeChatView] No provider found for '${this.providerName}'`);
			const errorContent = 'No AI provider configured. Please set up a provider first.';
			this.messages.push({ role: 'assistant', content: errorContent });
			this.appendMessageElement('assistant', errorContent);
			return;
		}

		this.isStreaming = true;
		this.sendButton.classList.add('disabled');
		const { element: assistantEl, contentElement: contentEl } = this.appendMessageElement('assistant', '');
		assistantEl.classList.add('streaming');

		// Resolve context and build messages with context prepended
		const resolved = this.forgeConfigService.resolveModel(this.providerName, this.model);
		const contextBudget = resolved?.contextBudget ?? 8000;
		const budget = await this.forgeContextService.resolveContextPrompt(this.panePosition, contextBudget);
		const messagesWithContext = this.buildMessagesWithContext(budget.items);

		let fullContent = '';
		try {
			for await (const chunk of provider.stream({
				model: this.model,
				messages: messagesWithContext,
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

	getMessages(): AIMessage[] {
		return [...this.messages];
	}

	getConversationId(): string {
		return this.currentConversationId;
	}

	getPanePosition(): PanePosition | undefined {
		return this.panePosition;
	}

	setPanePosition(position: PanePosition | undefined): void {
		this.panePosition = position;
	}

	private handleAtTrigger(): void {
		const value = this.inputArea.value;
		const cursorPos = this.inputArea.selectionStart;

		// Check if the character just typed is @ and it's at the start or preceded by whitespace
		if (cursorPos > 0 && value[cursorPos - 1] === '@') {
			const charBefore = cursorPos > 1 ? value[cursorPos - 2] : ' ';
			if (charBefore === ' ' || charBefore === '\n' || cursorPos === 1) {
				// Remove the @ character from the input
				this.inputArea.value = value.slice(0, cursorPos - 1) + value.slice(cursorPos);
				this.inputArea.selectionStart = cursorPos - 1;
				this.inputArea.selectionEnd = cursorPos - 1;

				this.forgeContextService.showContextPicker(this.panePosition).catch(error => {
					this.logService.warn('[ForgeChatView] Context picker failed', error);
				});
			}
		}
	}

	private renderContextChips(): void {
		this._chipListenerStore.clear();
		this.contextChipsContainer.textContent = '';
		const chips = this.forgeContextService.getContextChips(this.panePosition);

		for (const chip of chips) {
			const chipEl = $('.forge-context-chip');
			chipEl.dataset.type = chip.item.type;
			if (chip.automatic) {
				chipEl.classList.add('automatic');
			}

			const typeIcon = append(chipEl, $('span.forge-context-chip-type'));
			typeIcon.textContent = this.getTypeIcon(chip.item.type);

			const label = append(chipEl, $('span.label'));
			label.textContent = chip.item.label;

			const removeBtn = append(chipEl, $('button.remove'));
			removeBtn.textContent = '\u00d7'; // multiplication sign as close icon
			removeBtn.title = 'Remove context';
			this._chipListenerStore.add(addDisposableListener(removeBtn, EventType.CLICK, () => {
				this.forgeContextService.removeContextChip(this.panePosition, chip.item);
			}));

			append(this.contextChipsContainer, chipEl);
		}
	}

	private getTypeIcon(type: ForgeContextType): string {
		switch (type) {
			case ForgeContextType.File: return '\u{1F4C4}';
			case ForgeContextType.Selection: return '\u{1F4CB}';
			case ForgeContextType.GitDiff: return '\u{1F504}';
			case ForgeContextType.Symbol: return '\u{1F3F7}';
			case ForgeContextType.PaneHistory: return '\u{1F4AC}';
			case ForgeContextType.ActiveEditor: return '\u{270F}';
			default: return '\u{1F4CE}';
		}
	}

	private buildMessagesWithContext(contextChips: ForgeContextChip[]): AIMessage[] {
		if (contextChips.length === 0) {
			return [...this.messages];
		}

		const contextParts: string[] = [];
		for (const chip of contextChips) {
			const item = chip.item;

			// For pane history chips with empty content, insert a cross-pane reference
			if (item.type === ForgeContextType.PaneHistory && item.sourcePanePosition && !item.content) {
				const posLabel = item.sourcePanePosition.toUpperCase();
				contextParts.push(`<context pane="${posLabel}">Conversation reference from pane ${posLabel}</context>`);
			} else {
				contextParts.push(formatContextItem(item));
			}
		}

		const contextBlock = `<context>\n${contextParts.join('\n')}\n</context>`;

		// Prepend context as a system-like message at the start
		const contextMessage: AIMessage = {
			role: 'user',
			content: `${contextBlock}\n\nThe above context was attached by the user for reference.`,
		};

		// Insert context message before the conversation, after any existing system context
		return [contextMessage, ...this.messages];
	}

	layout(_width: number, _height: number): void {
		// Reserved for future resize handling
	}
}
