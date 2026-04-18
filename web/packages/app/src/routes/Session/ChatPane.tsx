import {
  type Component,
  createSignal,
  createEffect,
  For,
  Show,
  createMemo,
} from 'solid-js';
import { invoke } from '../../lib/tauri';
import { activeSessionId } from '../../stores/session';
import {
  getMessagesState,
  setAwaitingResponse,
  type ChatTurn,
} from '../../stores/messages';
import './ChatPane.css';

// ---------------------------------------------------------------------------
// ToolCallCard — inline tool call card (F-026)
// ---------------------------------------------------------------------------

const ToolCallCard: Component<{ turn: Extract<ChatTurn, { type: 'tool_placeholder' }> }> = (props) => (
  <div
    class="tool-placeholder"
    data-testid={`tool-call-card-${props.turn.tool_call_id}`}
    classList={{ 'tool-placeholder--completed': props.turn.status === 'completed' }}
  >
    <span class="tool-placeholder__icon" aria-hidden="true">⚙</span>
    <span class="tool-placeholder__name">{props.turn.tool_name}</span>
    <span class="tool-placeholder__status">
      {props.turn.status}
    </span>
  </div>
);

// ---------------------------------------------------------------------------
// Message turn renderers
// ---------------------------------------------------------------------------

const UserBubble: Component<{ turn: Extract<ChatTurn, { type: 'user' }> }> = (props) => (
  <article class="turn turn--user">
    <header class="turn__author">● you</header>
    <p class="turn__body">{props.turn.text}</p>
  </article>
);

const AssistantBubble: Component<{ turn: Extract<ChatTurn, { type: 'assistant' }> }> = (props) => (
  <article class="turn turn--assistant">
    <header class="turn__author">● assistant</header>
    <p class="turn__body">
      {props.turn.text}
      <Show when={props.turn.isStreaming}>
        <span class="streaming-cursor" data-testid="streaming-cursor" aria-hidden="true" />
      </Show>
    </p>
  </article>
);

const ErrorTurn: Component<{ turn: Extract<ChatTurn, { type: 'error' }> }> = (props) => (
  <div class="turn turn--error" role="alert">
    <span class="turn__error-icon" aria-hidden="true">!</span>
    <span class="turn__error-message">{props.turn.message}</span>
  </div>
);

// ---------------------------------------------------------------------------
// Composer
// ---------------------------------------------------------------------------

const Composer: Component<{ disabled: boolean; onSend: (text: string) => void }> = (props) => {
  const [text, setText] = createSignal('');

  const handleKeyDown = (e: KeyboardEvent) => {
    if (e.key !== 'Enter') return;

    // Option B: modifier keys = newline, bare Enter = send
    if (e.shiftKey || e.ctrlKey || e.metaKey) return;

    e.preventDefault();
    const value = text().trim();
    if (!value) return;
    props.onSend(value);
    setText('');
  };

  return (
    <div class="composer">
      <textarea
        class="composer__textarea"
        data-testid="composer-textarea"
        placeholder="Ask, refine, or @-reference context"
        disabled={props.disabled}
        value={text()}
        onInput={(e) => setText(e.currentTarget.value)}
        onKeyDown={handleKeyDown}
        rows={3}
      />
      <div class="composer__bar">
        <span class="composer__hints">
          <span class="composer__hint">@ for context</span>
          <span class="composer__hint">/ for commands</span>
        </span>
        <div class="composer__actions">
          <Show when={props.disabled}>
            <button type="button" class="composer__btn composer__btn--ghost">
              Stop
            </button>
          </Show>
          <span class="composer__send-hint">
            {props.disabled ? 'Streaming…' : 'Send ↵'}
          </span>
        </div>
      </div>
    </div>
  );
};

// ---------------------------------------------------------------------------
// ChatPane root
// ---------------------------------------------------------------------------

export const ChatPane: Component = () => {
  const sessionId = () => activeSessionId();
  const state = createMemo(() => {
    const id = sessionId();
    if (!id) return { turns: [], awaitingResponse: false, streamingMessageId: null };
    return getMessagesState(id);
  });

  let listRef: HTMLDivElement | undefined;
  // Track whether the user has scrolled up (released auto-pin)
  const [userScrolledUp, setUserScrolledUp] = createSignal(false);

  const scrollToBottom = () => {
    if (listRef) {
      listRef.scrollTop = listRef.scrollHeight;
    }
  };

  // Auto-scroll to bottom when turns change or streaming progresses,
  // as long as the user hasn't scrolled up to read earlier content.
  createEffect(() => {
    const { turns, streamingMessageId } = state();
    // Access both to establish reactive dependencies; values unused directly.
    const _len = turns.length;
    const _sid = streamingMessageId;
    if (!userScrolledUp()) {
      scrollToBottom();
    }
  });

  const handleListScroll = () => {
    if (!listRef) return;
    const atBottom = listRef.scrollHeight - listRef.scrollTop - listRef.clientHeight < 8;
    setUserScrolledUp(!atBottom);
  };

  const handleSend = (text: string) => {
    const id = sessionId();
    if (!id) return;
    setAwaitingResponse(id, true);
    // Re-pin on new user message
    setUserScrolledUp(false);
    void invoke('session_send_message', { sessionId: id, text });
  };

  return (
    <section class="chat-pane" data-testid="chat-pane" aria-label="Chat pane">
      <span class="chat-pane__type-label">CHAT</span>
      {/* Streaming indicator shown while awaiting a response */}
      <Show when={state().awaitingResponse}>
        <div class="chat-pane__streaming-indicator" data-testid="streaming-indicator" aria-live="polite">
          <span class="streaming-cursor" aria-hidden="true" />
          <span>Awaiting response…</span>
        </div>
      </Show>

      {/* Message list */}
      <div
        class="chat-pane__messages"
        data-testid="message-list"
        data-autoscroll=""
        role="log"
        aria-live="polite"
        ref={listRef}
        onScroll={handleListScroll}
      >
        <For each={state().turns}>
          {(turn) => {
            switch (turn.type) {
              case 'user':
                return <UserBubble turn={turn} />;
              case 'assistant':
                return <AssistantBubble turn={turn} />;
              case 'tool_placeholder':
                return <ToolCallCard turn={turn} />;
              case 'error':
                return <ErrorTurn turn={turn} />;
            }
          }}
        </For>
      </div>

      {/* Composer */}
      <Composer disabled={state().awaitingResponse} onSend={handleSend} />
    </section>
  );
};
