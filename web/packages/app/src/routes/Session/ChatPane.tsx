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
  pushEvent,
  setAwaitingResponse,
  type ChatTurn,
} from '../../stores/messages';
import type { SessionId } from '@forge/ipc';
import {
  getApprovalWhitelist,
  addWhitelistEntry,
  revokeWhitelistEntry,
  matchWhitelistKey,
} from '../../stores/approvals';
import { ApprovalPrompt } from '../../components/ApprovalPrompt/ApprovalPrompt';
import { WhitelistedPill } from '../../components/ApprovalPrompt/WhitelistedPill';
import type { ApprovalScope } from '@forge/ipc';
import './ChatPane.css';

// ---------------------------------------------------------------------------
// invoke rejection helper (F-079)
// ---------------------------------------------------------------------------

/**
 * Surface a rejected `invoke()` as an inline `error` turn in the chat. The
 * `Error` event handler in the messages store (see `stores/messages.ts`) also
 * clears `awaitingResponse` and `streamingMessageId`, which is what rolls back
 * the optimistic disable performed by `handleSend` before the call. Routing
 * every command-rejection through this single sink keeps the user-feedback
 * shape consistent across approve/reject/send call sites.
 */
function reportInvokeError(sessionId: SessionId, command: string, err: unknown): void {
  const detail = err instanceof Error ? err.message : String(err);
  pushEvent(sessionId, { kind: 'Error', message: `${command} failed: ${detail}` });
}

// ---------------------------------------------------------------------------
// ToolCallCard — inline tool call card with optional approval prompt (F-026/F-027)
// ---------------------------------------------------------------------------

/**
 * One-line arg summary for the collapsed tool-call card (F-041). Path-taking
 * tools (fs.read / fs.write / fs.edit / shell.exec) render their `path` whole;
 * anything else gets `JSON.stringify(args)` capped near 60 chars with an
 * ellipsis. Unparseable args_json returns null — the component skips the span
 * via `<Show>`, so malformed payloads never crash the card.
 */
function summarizeArgs(argsJson: string): string | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(argsJson);
  } catch {
    return null;
  }
  if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
    const path = (parsed as Record<string, unknown>)['path'];
    if (typeof path === 'string' && path.length > 0) {
      return path;
    }
  }
  const stringified = JSON.stringify(parsed);
  if (stringified === undefined) return null;
  const MAX = 60;
  return stringified.length > MAX ? stringified.slice(0, MAX - 1) + '…' : stringified;
}

const ToolCallCard: Component<{ turn: Extract<ChatTurn, { type: 'tool_placeholder' }> }> = (
  props,
) => {
  let cardRef: HTMLDivElement | undefined;
  const sessionId = () => activeSessionId();

  // Check whitelist on each render
  const whitelistKey = createMemo(() => {
    const id = sessionId();
    if (!id) return null;
    if (props.turn.status !== 'awaiting-approval') return null;
    let path = '';
    try {
      const args = JSON.parse(props.turn.args_json) as Record<string, unknown>;
      if (typeof args['path'] === 'string') path = args['path'];
    } catch {
      // ignore
    }
    const wl = getApprovalWhitelist(id);
    const keys = new Set(Object.keys(wl.entries));
    return matchWhitelistKey(keys, props.turn.tool_name, path);
  });

  const whitelistLabel = createMemo(() => {
    const id = sessionId();
    const key = whitelistKey();
    if (!id || !key) return null;
    return getApprovalWhitelist(id).entries[key] ?? null;
  });

  // Auto-approve when whitelist matches
  createEffect(() => {
    const id = sessionId();
    const key = whitelistKey();
    if (!id || !key || props.turn.status !== 'awaiting-approval') return;
    invoke('session_approve_tool', {
      sessionId: id,
      toolCallId: props.turn.tool_call_id,
      // Derive scope from key prefix
      scope: key.startsWith('tool:')
        ? ('ThisTool' as ApprovalScope)
        : key.startsWith('pattern:')
          ? ('ThisPattern' as ApprovalScope)
          : ('ThisFile' as ApprovalScope),
    }).catch((err) => reportInvokeError(id, 'session_approve_tool', err));
  });

  const handleApprove = (scope: ApprovalScope, pattern?: string) => {
    const id = sessionId();
    if (!id) return;

    // Record whitelist for scopes > Once
    if (scope !== 'Once') {
      let path = '';
      try {
        const args = JSON.parse(props.turn.args_json) as Record<string, unknown>;
        if (typeof args['path'] === 'string') path = args['path'];
      } catch {
        // ignore
      }
      addWhitelistEntry(id, scope, props.turn.tool_name, path, pattern);
    }

    invoke('session_approve_tool', {
      sessionId: id,
      toolCallId: props.turn.tool_call_id,
      scope,
    }).catch((err) => reportInvokeError(id, 'session_approve_tool', err));
  };

  const handleReject = () => {
    const id = sessionId();
    if (!id) return;
    invoke('session_reject_tool', {
      sessionId: id,
      toolCallId: props.turn.tool_call_id,
    }).catch((err) => reportInvokeError(id, 'session_reject_tool', err));
  };

  const handleRevoke = () => {
    const id = sessionId();
    const key = whitelistKey();
    if (!id || !key) return;
    revokeWhitelistEntry(id, key);
  };

  return (
    <div
      class="tool-placeholder"
      data-testid={`tool-call-card-${props.turn.tool_call_id}`}
      classList={{
        'tool-placeholder--completed': props.turn.status === 'completed',
        'tool-placeholder--awaiting': props.turn.status === 'awaiting-approval',
      }}
      tabIndex={props.turn.status === 'awaiting-approval' ? 0 : undefined}
      ref={cardRef}
    >
      <div class="tool-placeholder__header">
        <span class="tool-placeholder__icon" aria-hidden="true">
          ⚙
        </span>
        <span class="tool-placeholder__name">{props.turn.tool_name}</span>

        {/* One-line arg summary — path for path-taking tools, otherwise a
            short stringified JSON. Skipped when args_json is unparseable. */}
        <Show when={summarizeArgs(props.turn.args_json)}>
          {(summary) => (
            <span
              class="tool-placeholder__args"
              data-testid={`tool-call-args-${props.turn.tool_call_id}`}
            >
              {summary()}
            </span>
          )}
        </Show>

        {/* Whitelisted pill when auto-approved */}
        <Show when={whitelistKey() !== null && whitelistLabel() !== null}>
          <WhitelistedPill label={whitelistLabel()!} onRevoke={handleRevoke} />
        </Show>

        {/* Status label (hidden when awaiting — prompt fills that role) */}
        <Show when={props.turn.status !== 'awaiting-approval' || whitelistKey() !== null}>
          <span class="tool-placeholder__status">{props.turn.status}</span>
        </Show>
      </div>

      {/* Inline approval prompt */}
      <Show
        when={
          props.turn.status === 'awaiting-approval' &&
          props.turn.preview !== undefined &&
          whitelistKey() === null
        }
      >
        <ApprovalPrompt
          toolCallId={props.turn.tool_call_id}
          toolName={props.turn.tool_name}
          argsJson={props.turn.args_json}
          preview={props.turn.preview!}
          containerRef={cardRef!}
          onApprove={handleApprove}
          onReject={handleReject}
        />
      </Show>
    </div>
  );
};

// ---------------------------------------------------------------------------
// Message turn renderers
// ---------------------------------------------------------------------------

const UserBubble: Component<{ turn: Extract<ChatTurn, { type: 'user' }> }> = (props) => (
  <article class="turn turn--user">
    <header class="turn__author">● you</header>
    <p class="turn__body">{props.turn.text}</p>
  </article>
);

const AssistantBubble: Component<{ turn: Extract<ChatTurn, { type: 'assistant' }> }> = (
  props,
) => (
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
    // On rejection, the Error event handler in the messages store rolls back
    // both `awaitingResponse` and `streamingMessageId`, re-enabling the
    // composer and surfacing the failure as an inline error turn.
    invoke('session_send_message', { sessionId: id, text }).catch((err) =>
      reportInvokeError(id, 'session_send_message', err),
    );
  };

  return (
    <section class="chat-pane" data-testid="chat-pane" aria-label="Chat pane">
      <span class="chat-pane__type-label">CHAT</span>
      {/* Streaming indicator shown while awaiting a response */}
      <Show when={state().awaitingResponse}>
        <div
          class="chat-pane__streaming-indicator"
          data-testid="streaming-indicator"
          aria-live="polite"
        >
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

      {/* Composer — disabled while we are awaiting the first token OR a
          stream is in flight. The store clears `awaitingResponse` on the
          first AssistantDelta but leaves `streamingMessageId` set until
          AssistantMessage(stream_finalised: true), so both must read
          falsy before the composer re-enables. */}
      <Composer
        disabled={state().awaitingResponse || state().streamingMessageId !== null}
        onSend={handleSend}
      />
    </section>
  );
};
