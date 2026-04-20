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
import {
  ContextPicker,
  detectAtTrigger,
  type PickerResult,
  type ContextCategory,
} from '../../components/ContextPicker';
import { ContextChip } from '../../components/ContextChip';
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
 * ellipsis.
 *
 * F-080 item 6: `args_json` is produced exclusively by `fromRustEvent`
 * (`ipc/events.ts`) via `JSON.stringify(ev['args'] ?? null)`, so it is
 * guaranteed to be valid JSON at this boundary. The previous defensive
 * `try { JSON.parse } catch { return null }` was cognitive overhead with no
 * risk reduction; relying on the boundary contract collapses three sites in
 * this file to a single `JSON.parse` per call.
 */
function summarizeArgs(argsJson: string): string | null {
  const parsed = JSON.parse(argsJson) as unknown;
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

/**
 * Extract a `path` field from a tool call's `args_json` if present.
 *
 * F-080 item 6: shares the boundary contract documented on `summarizeArgs` —
 * `args_json` is always valid JSON. Returns `''` when the parsed value is not
 * an object with a string `path` field (e.g. `null`, an array, or a
 * non-path-taking tool).
 */
function extractPath(argsJson: string): string {
  const parsed = JSON.parse(argsJson) as unknown;
  if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
    const path = (parsed as Record<string, unknown>)['path'];
    if (typeof path === 'string') return path;
  }
  return '';
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
    const path = extractPath(props.turn.args_json);
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
      const path = extractPath(props.turn.args_json);
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

/// F-080 item 5: composer-side message-byte cap. The Rust side
/// (`forge-shell::ipc::session_send_message`) enforces a 128 KiB cap on the
/// UTF-8 byte length of `text`; capping at 100 KiB on the TS side gives the
/// user feedback before the IPC round trip and stays comfortably below the
/// backend cap so a marginal extra prompt token does not race the boundary.
/// `TextEncoder` measures UTF-8 bytes (matching the Rust check) — `String.length`
/// would return UTF-16 code units and be wrong for non-BMP input.
export const MAX_COMPOSER_BYTES = 100 * 1024;

function utf8ByteLength(text: string): number {
  return new TextEncoder().encode(text).length;
}

interface InsertedChip {
  /** Stable id so SolidJS can reconcile chip list updates. */
  id: string;
  category: ContextCategory;
  label: string;
  value: string;
}

/**
 * Remove the active `@text` span from the textarea content when a picker
 * result is selected (F-141). Exported for direct unit testing — the
 * integration in Composer calls this while also appending the chip to the
 * `ctx-chips` row and repositioning the caret.
 *
 * The span runs from `triggerStart` (index of the `@`) to `caret`. The
 * result is the concatenation of the text before `triggerStart` and
 * after `caret`, plus the new caret position at the join.
 */
export function removeAtSpan(
  text: string,
  triggerStart: number,
  caret: number,
): { text: string; caret: number } {
  const before = text.slice(0, triggerStart);
  const after = text.slice(caret);
  return { text: before + after, caret: before.length };
}

export interface ComposerProps {
  disabled: boolean;
  onSend: (text: string) => void;
  /**
   * Optional category-indexed items forwarded to the ContextPicker. F-141
   * ships with this undefined (the picker shows empty tabs). F-142 will
   * wire a resolver on top of this prop. Exposed now so component tests
   * can drive the end-to-end "type @ → pick → chip appears" flow.
   */
  items?: Partial<Record<ContextCategory, PickerResult[]>>;
}

export const Composer: Component<ComposerProps> = (props) => {
  const [text, setText] = createSignal('');
  const [caret, setCaret] = createSignal(0);
  const [chips, setChips] = createSignal<InsertedChip[]>([]);
  // F-141: Esc dismisses the picker but must *retain* the typed `@text`
  // (spec §7 "close, retain typed text"). We track the dismissed `@`-span
  // start so the trigger re-opens only after the user edits the text — not
  // just from a caret move back into the same span.
  const [dismissedAt, setDismissedAt] = createSignal<number | null>(null);
  // F-141: the ContextPicker pops open while an active `@token` sits at the
  // caret. `detectAtTrigger` drives this — whenever the caret or text moves,
  // we recompute whether we're still in a trigger.
  const trigger = createMemo(() => {
    const match = detectAtTrigger(text(), caret());
    if (!match) return null;
    if (dismissedAt() === match.start) return null;
    return match;
  });
  const pickerOpen = createMemo(() => trigger() !== null);
  // Anchor rect used by the ContextPicker for placement. Re-measured on open
  // and on viewport resize.
  const [anchorRect, setAnchorRect] = createSignal({
    top: 0,
    bottom: 0,
    left: 0,
    right: 0,
  });

  let textareaRef: HTMLTextAreaElement | undefined;
  let composerRef: HTMLDivElement | undefined;

  // Byte length of the *trimmed* value because that is what we actually send.
  const trimmedByteLength = createMemo(() => utf8ByteLength(text().trim()));
  const overCap = createMemo(() => trimmedByteLength() > MAX_COMPOSER_BYTES);

  const measureAnchor = () => {
    if (!composerRef) return;
    const r = composerRef.getBoundingClientRect();
    setAnchorRect({ top: r.top, bottom: r.bottom, left: r.left, right: r.right });
  };

  createEffect(() => {
    if (pickerOpen()) {
      measureAnchor();
    }
  });

  const handleInput = (e: InputEvent & { currentTarget: HTMLTextAreaElement }) => {
    const t = e.currentTarget.value;
    setText(t);
    setCaret(e.currentTarget.selectionStart ?? t.length);
    // Any text edit re-arms the trigger — the user's dismiss decision only
    // applied to the span they dismissed.
    setDismissedAt(null);
  };

  const handleSelect = (e: Event & { currentTarget: HTMLTextAreaElement }) => {
    setCaret(e.currentTarget.selectionStart ?? text().length);
  };

  const handleSend = () => {
    const value = text().trim();
    if (!value) return;
    if (utf8ByteLength(value) > MAX_COMPOSER_BYTES) return;
    props.onSend(value);
    setText('');
    setCaret(0);
    // Chips persist across sends intentionally — clearing them on send is
    // F-142 territory once backend context blocks wire up.
  };

  const handleKeyDown = (e: KeyboardEvent) => {
    // When the picker is open it owns Arrow/Enter/Tab/Escape — stop those
    // from reaching the textarea's send handler. The picker itself installs
    // a capturing window listener, so we just need to make sure the
    // textarea-level Enter-to-send doesn't fire while the picker is up.
    if (pickerOpen()) {
      if (
        e.key === 'Enter' ||
        e.key === 'Escape' ||
        e.key === 'Tab' ||
        e.key === 'ArrowUp' ||
        e.key === 'ArrowDown'
      ) {
        return;
      }
    }
    if (e.key !== 'Enter') return;
    // Option B: modifier keys = newline, bare Enter = send
    if (e.shiftKey || e.ctrlKey || e.metaKey) return;
    e.preventDefault();
    handleSend();
  };

  const replaceAtSpan = (result: PickerResult) => {
    // Remove the `@text` span from the textarea and append a chip to the
    // `ctx-chips` row. The span runs from `trigger.start` to the caret.
    const t = text();
    const c = caret();
    const match = trigger();
    if (!match) {
      // Picker was open with no active trigger — append the chip and move
      // on; the textarea stays as-is.
      setChips((prev) => [
        ...prev,
        {
          id: `chip-${Date.now()}-${prev.length}`,
          category: result.category,
          label: result.label,
          value: result.value,
        },
      ]);
      return;
    }
    const { text: next, caret: nextCaret } = removeAtSpan(t, match.start, c);
    setText(next);
    setCaret(nextCaret);
    setDismissedAt(null);
    setChips((prev) => [
      ...prev,
      {
        id: `chip-${Date.now()}-${prev.length}`,
        category: result.category,
        label: result.label,
        value: result.value,
      },
    ]);
    // Move the real textarea caret to the join point so subsequent typing
    // continues where the `@span` was.
    queueMicrotask(() => {
      if (textareaRef) {
        textareaRef.selectionStart = nextCaret;
        textareaRef.selectionEnd = nextCaret;
        textareaRef.focus();
      }
    });
  };

  const dismissPicker = () => {
    // "Esc: close, retain typed text" — the `@text` stays in the textarea.
    // We suppress re-opening the picker for this particular `@`-span; any
    // subsequent text edit (via handleInput) clears the suppression.
    const match = trigger();
    if (match) {
      setDismissedAt(match.start);
      queueMicrotask(() => {
        if (textareaRef) {
          textareaRef.focus();
        }
      });
    }
  };

  const dismissChip = (id: string) => {
    setChips((prev) => prev.filter((c) => c.id !== id));
  };

  return (
    <div class="composer" ref={composerRef}>
      {/* ctx-chips row — chips inserted from the picker live here. The spec
          places it above the textarea so chips are visually attached to
          the message being composed. */}
      <div
        class="composer__ctx-chips"
        data-testid="ctx-chips"
        classList={{ 'composer__ctx-chips--empty': chips().length === 0 }}
      >
        <For each={chips()}>
          {(chip) => (
            <ContextChip
              category={chip.category}
              label={chip.label}
              onDismiss={() => dismissChip(chip.id)}
            />
          )}
        </For>
      </div>
      <textarea
        class="composer__textarea"
        data-testid="composer-textarea"
        placeholder="Ask, refine, or @-reference context"
        disabled={props.disabled}
        value={text()}
        onInput={handleInput}
        onSelect={handleSelect}
        onClick={handleSelect}
        onKeyUp={handleSelect}
        onKeyDown={handleKeyDown}
        rows={3}
        aria-invalid={overCap() ? true : undefined}
        ref={textareaRef}
      />
      <Show when={pickerOpen()}>
        <ContextPicker
          query={trigger()!.query}
          anchorRect={anchorRect()}
          {...(props.items ? { items: props.items } : {})}
          onPick={replaceAtSpan}
          onDismiss={dismissPicker}
        />
      </Show>
      <div class="composer__bar">
        <span class="composer__hints">
          <Show
            when={overCap()}
            fallback={
              <>
                <span class="composer__hint">@ for context</span>
                <span class="composer__hint">/ for commands</span>
              </>
            }
          >
            <span
              class="composer__hint composer__hint--warning"
              data-testid="composer-overflow-warning"
              role="status"
            >
              Message is {trimmedByteLength().toLocaleString()} bytes — over the{' '}
              {MAX_COMPOSER_BYTES.toLocaleString()}-byte limit. Trim before sending.
            </span>
          </Show>
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
