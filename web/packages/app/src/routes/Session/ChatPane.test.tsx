import { afterEach, describe, expect, it, vi, beforeEach } from 'vitest';
import { render, fireEvent } from '@solidjs/testing-library';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import type { SessionId } from '@forge/ipc';

// --- Store imports ---
import {
  pushEvent,
  setAwaitingResponse,
  resetMessagesStore,
} from '../../stores/messages';
import { setActiveSessionId } from '../../stores/session';
import { resetApprovalsStore } from '../../stores/approvals';
import { setInvokeForTesting } from '../../lib/tauri';
import { ChatPane, Composer, MAX_COMPOSER_BYTES, removeAtSpan } from './ChatPane';

const SID = 'session-chat-test' as SessionId;
const invokeMock = vi.fn();

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(undefined);
  setInvokeForTesting(invokeMock as never);
  resetMessagesStore();
  resetApprovalsStore();
  setActiveSessionId(SID);
});

afterEach(() => {
  setInvokeForTesting(null);
});

describe('ChatPane rendering', () => {
  it('renders the message list container', () => {
    const { getByTestId } = render(() => <ChatPane />);
    expect(getByTestId('message-list')).toBeInTheDocument();
  });

  it('renders the composer textarea', () => {
    const { getByTestId } = render(() => <ChatPane />);
    expect(getByTestId('composer-textarea')).toBeInTheDocument();
  });

  it('renders a user message turn', () => {
    pushEvent(SID, { kind: 'UserMessage', text: 'Hello world', message_id: 'u1' });
    const { getByText } = render(() => <ChatPane />);
    expect(getByText('Hello world')).toBeInTheDocument();
  });

  it('renders an assistant message turn', () => {
    pushEvent(SID, { kind: 'AssistantMessage', text: 'Hi there', message_id: 'a1' });
    const { getByText } = render(() => <ChatPane />);
    expect(getByText('Hi there')).toBeInTheDocument();
  });

  it('renders a streaming assistant turn with blinking cursor', () => {
    pushEvent(SID, { kind: 'AssistantDelta', delta: 'Typing...', message_id: 'a2' });
    const { getByTestId } = render(() => <ChatPane />);
    expect(getByTestId('streaming-cursor')).toBeInTheDocument();
  });

  it('does not render streaming cursor on completed messages', () => {
    pushEvent(SID, { kind: 'AssistantMessage', text: 'Done', message_id: 'a3' });
    const { queryByTestId } = render(() => <ChatPane />);
    expect(queryByTestId('streaming-cursor')).not.toBeInTheDocument();
  });

  it('renders a tool call card', () => {
    pushEvent(SID, {
      kind: 'ToolCallStarted',
      tool_call_id: 'tc-1',
      tool_name: 'fs.read',
      args_json: '{}',
    });
    const { getByTestId } = render(() => <ChatPane />);
    expect(getByTestId('tool-call-card-tc-1')).toBeInTheDocument();
  });

  // F-041: collapsed tool call card renders a one-line arg summary to the
  // right of the tool name. Path-taking tools show the path; other tools
  // show a short stringified JSON; unparseable args render no summary.
  it('renders the path as the arg summary when args_json has a path field', () => {
    pushEvent(SID, {
      kind: 'ToolCallStarted',
      tool_call_id: 'tc-path',
      tool_name: 'fs.read',
      args_json: JSON.stringify({ path: 'readable.txt' }),
    });
    const { getByTestId } = render(() => <ChatPane />);
    const card = getByTestId('tool-call-card-tc-path');
    expect(card).toHaveTextContent('readable.txt');
  });

  it('falls back to stringified args truncated to ~60 chars when no path field', () => {
    const bigArgs = { query: 'x'.repeat(120), scope: 'everything' };
    pushEvent(SID, {
      kind: 'ToolCallStarted',
      tool_call_id: 'tc-nopath',
      tool_name: 'search',
      args_json: JSON.stringify(bigArgs),
    });
    const { getByTestId } = render(() => <ChatPane />);
    const card = getByTestId('tool-call-card-tc-nopath');
    // Contains the leading tokens of the stringified JSON…
    expect(card).toHaveTextContent('"query"');
    // …but is bounded by the ~60-char cap, not the full 100+ char payload.
    expect(card.textContent ?? '').not.toContain('x'.repeat(80));
  });

  // F-080 item 6: the prior "unparseable args_json" test was deleted because
  // `args_json` is produced exclusively by `fromRustEvent`
  // (`JSON.stringify(ev['args'] ?? null)`) and is contractually valid JSON at
  // the ChatPane boundary. The defensive `try/catch` blocks that handled
  // unparseable input were removed in F-080; pinning this test would have
  // required keeping a dead code path in the component just to satisfy an
  // unreachable input shape.

  it('renders an error turn inline', () => {
    pushEvent(SID, { kind: 'Error', message: 'ECONNREFUSED 127.0.0.1:11434' });
    const { getByText } = render(() => <ChatPane />);
    expect(getByText(/ECONNREFUSED/)).toBeInTheDocument();
  });
});

describe('Composer keyboard behavior (Option B)', () => {
  it('sends message on Enter key', async () => {
    invokeMock.mockResolvedValue(undefined);
    const { getByTestId } = render(() => <ChatPane />);
    const textarea = getByTestId('composer-textarea') as HTMLTextAreaElement;

    fireEvent.input(textarea, { target: { value: 'send this' } });
    fireEvent.keyDown(textarea, { key: 'Enter', code: 'Enter', shiftKey: false, ctrlKey: false, metaKey: false });

    expect(invokeMock).toHaveBeenCalledWith('session_send_message', {
      sessionId: SID,
      text: 'send this',
    });
  });

  it('inserts newline on Shift+Enter instead of sending', () => {
    const { getByTestId } = render(() => <ChatPane />);
    const textarea = getByTestId('composer-textarea') as HTMLTextAreaElement;

    fireEvent.input(textarea, { target: { value: 'line1' } });
    fireEvent.keyDown(textarea, { key: 'Enter', code: 'Enter', shiftKey: true });

    expect(invokeMock).not.toHaveBeenCalled();
  });

  it('inserts newline on Ctrl+Enter instead of sending', () => {
    const { getByTestId } = render(() => <ChatPane />);
    const textarea = getByTestId('composer-textarea') as HTMLTextAreaElement;

    fireEvent.input(textarea, { target: { value: 'line1' } });
    fireEvent.keyDown(textarea, { key: 'Enter', code: 'Enter', ctrlKey: true });

    expect(invokeMock).not.toHaveBeenCalled();
  });

  it('inserts newline on Cmd+Enter instead of sending', () => {
    const { getByTestId } = render(() => <ChatPane />);
    const textarea = getByTestId('composer-textarea') as HTMLTextAreaElement;

    fireEvent.input(textarea, { target: { value: 'line1' } });
    fireEvent.keyDown(textarea, { key: 'Enter', code: 'Enter', metaKey: true });

    expect(invokeMock).not.toHaveBeenCalled();
  });

  it('clears the textarea after sending', async () => {
    invokeMock.mockResolvedValue(undefined);
    const { getByTestId } = render(() => <ChatPane />);
    const textarea = getByTestId('composer-textarea') as HTMLTextAreaElement;

    fireEvent.input(textarea, { target: { value: 'hello' } });
    fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false, ctrlKey: false, metaKey: false });

    expect(textarea.value).toBe('');
  });

  it('does not send an empty message', () => {
    const { getByTestId } = render(() => <ChatPane />);
    const textarea = getByTestId('composer-textarea') as HTMLTextAreaElement;

    fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false, ctrlKey: false, metaKey: false });

    expect(invokeMock).not.toHaveBeenCalled();
  });
});

describe('Composer disabled state', () => {
  it('disables the textarea while awaiting response', () => {
    setAwaitingResponse(SID, true);
    const { getByTestId } = render(() => <ChatPane />);
    const textarea = getByTestId('composer-textarea') as HTMLTextAreaElement;
    expect(textarea).toBeDisabled();
  });

  it('enables the textarea when not awaiting', () => {
    setAwaitingResponse(SID, false);
    const { getByTestId } = render(() => <ChatPane />);
    const textarea = getByTestId('composer-textarea') as HTMLTextAreaElement;
    expect(textarea).not.toBeDisabled();
  });

  // F-040: the AssistantDelta handler clears `awaitingResponse` to false on the
  // first token, so a predicate based only on awaitingResponse re-enables the
  // composer mid-stream. The composer must stay disabled until the stream
  // finalises (streamingMessageId !== null), then re-enable on the final.
  it('stays disabled across awaiting → delta → final, then re-enables', () => {
    setAwaitingResponse(SID, true);
    const { getByTestId } = render(() => <ChatPane />);
    const textarea = getByTestId('composer-textarea') as HTMLTextAreaElement;
    expect(textarea).toBeDisabled();

    // First delta arrives — `awaitingResponse` flips to false in the store,
    // but `streamingMessageId` is set; composer must STILL be disabled.
    pushEvent(SID, { kind: 'AssistantDelta', delta: 'Hi', message_id: 'turn-1' });
    expect(textarea).toBeDisabled();

    // Mid-stream delta — still disabled.
    pushEvent(SID, { kind: 'AssistantDelta', delta: ' there.', message_id: 'turn-1' });
    expect(textarea).toBeDisabled();

    // Stream finalises — composer re-enables.
    pushEvent(SID, { kind: 'AssistantMessage', text: 'Hi there.', message_id: 'turn-1' });
    expect(textarea).not.toBeDisabled();
  });

  it('shows a streaming indicator while awaiting response', () => {
    setAwaitingResponse(SID, true);
    const { getByTestId } = render(() => <ChatPane />);
    expect(getByTestId('streaming-indicator')).toBeInTheDocument();
  });
});

// ---------------------------------------------------------------------------
// F-086: composer disabled-state CSS contract
// ---------------------------------------------------------------------------
//
// `component-principles.md` ("Buttons"): "Disabled buttons use iron-600
// background and text. Never reduce opacity on a button to show disabled
// state — opacity makes elements appear interactive." The rule applies to
// any interactive control, including the composer textarea while a stream
// is locked. jsdom does not resolve external stylesheets at render time, so
// the CSS source itself is the contract — assert against the source the
// way `check-tokens.test.ts` asserts against `tokens.css`.
describe('Composer disabled state CSS (F-086)', () => {
  const cssSource = readFileSync(
    resolve(__dirname, 'ChatPane.css'),
    'utf-8',
  );

  // Match `.composer__textarea:disabled { ... }` and capture the rule body.
  const ruleMatch = cssSource.match(
    /\.composer__textarea:disabled\s*\{([^}]*)\}/,
  );

  it('declares a `:disabled` rule for the composer textarea', () => {
    expect(ruleMatch).not.toBeNull();
  });

  it('does not reduce opacity to signal disabled (component-principles.md)', () => {
    const body = ruleMatch?.[1] ?? '';
    expect(body).not.toMatch(/\bopacity\s*:/);
  });

  it('uses --color-surface-2 for the locked background', () => {
    const body = ruleMatch?.[1] ?? '';
    expect(body).toMatch(/background\s*:\s*var\(--color-surface-2\)/);
  });

  it('uses --color-text-disabled for the locked text color', () => {
    const body = ruleMatch?.[1] ?? '';
    expect(body).toMatch(/color\s*:\s*var\(--color-text-disabled\)/);
  });

  it('keeps `cursor: not-allowed` to signal the locked state', () => {
    const body = ruleMatch?.[1] ?? '';
    expect(body).toMatch(/cursor\s*:\s*not-allowed/);
  });
});

describe('Auto-scroll behavior', () => {
  it('message list has data-autoscroll attribute for pinning', () => {
    const { getByTestId } = render(() => <ChatPane />);
    const list = getByTestId('message-list');
    expect(list).toHaveAttribute('data-autoscroll');
  });

  it('scrolls to bottom when a new streaming delta arrives', () => {
    const { getByTestId } = render(() => <ChatPane />);
    const list = getByTestId('message-list');

    // Mock scrollTop/scrollHeight so we can verify scrollTop was set
    Object.defineProperty(list, 'scrollHeight', { value: 500, configurable: true });
    Object.defineProperty(list, 'clientHeight', { value: 200, configurable: true });

    pushEvent(SID, { kind: 'AssistantDelta', delta: 'streaming text', message_id: 'stream-1' });

    // After a delta, the list should be pinned to bottom (scrollTop === scrollHeight)
    expect(list.scrollTop).toBe(500);
  });
});

// ---------------------------------------------------------------------------
// Inline approval prompt (F-027)
// ---------------------------------------------------------------------------

describe('Inline approval prompt', () => {
  const PREVIEW = { description: 'Edit file /src/foo.ts: 3 hunks, +47 -21' };
  const FS_EDIT_ARGS = JSON.stringify({ path: '/src/foo.ts', patch: '...' });

  it('renders the approval prompt when ToolCallApprovalRequested arrives', () => {
    pushEvent(SID, {
      kind: 'ToolCallApprovalRequested',
      tool_call_id: 'tc-ap-1',
      tool_name: 'fs.edit',
      args_json: FS_EDIT_ARGS,
      preview: PREVIEW,
    });
    const { getByTestId } = render(() => <ChatPane />);
    expect(getByTestId('approval-prompt')).toBeInTheDocument();
  });

  it('displays preview description inside the prompt', () => {
    pushEvent(SID, {
      kind: 'ToolCallApprovalRequested',
      tool_call_id: 'tc-ap-2',
      tool_name: 'fs.edit',
      args_json: FS_EDIT_ARGS,
      preview: PREVIEW,
    });
    const { getByTestId } = render(() => <ChatPane />);
    expect(getByTestId('approval-preview')).toHaveTextContent('Edit file /src/foo.ts');
  });

  it('invokes session_approve_tool with Once when approve is clicked', async () => {
    pushEvent(SID, {
      kind: 'ToolCallApprovalRequested',
      tool_call_id: 'tc-ap-3',
      tool_name: 'fs.edit',
      args_json: FS_EDIT_ARGS,
      preview: PREVIEW,
    });
    const { getByTestId } = render(() => <ChatPane />);
    fireEvent.click(getByTestId('approve-once-btn'));
    expect(invokeMock).toHaveBeenCalledWith('session_approve_tool', {
      sessionId: SID,
      toolCallId: 'tc-ap-3',
      scope: 'Once',
    });
  });

  it('invokes session_reject_tool when reject is clicked', () => {
    pushEvent(SID, {
      kind: 'ToolCallApprovalRequested',
      tool_call_id: 'tc-ap-4',
      tool_name: 'fs.edit',
      args_json: FS_EDIT_ARGS,
      preview: PREVIEW,
    });
    const { getByTestId } = render(() => <ChatPane />);
    fireEvent.click(getByTestId('reject-btn'));
    expect(invokeMock).toHaveBeenCalledWith('session_reject_tool', {
      sessionId: SID,
      toolCallId: 'tc-ap-4',
    });
  });

  it('invokes session_approve_tool with ThisFile from scope menu', () => {
    pushEvent(SID, {
      kind: 'ToolCallApprovalRequested',
      tool_call_id: 'tc-ap-5',
      tool_name: 'fs.edit',
      args_json: FS_EDIT_ARGS,
      preview: PREVIEW,
    });
    const { getByTestId } = render(() => <ChatPane />);
    fireEvent.click(getByTestId('approve-dropdown-btn'));
    fireEvent.click(getByTestId('scope-file-btn'));
    expect(invokeMock).toHaveBeenCalledWith('session_approve_tool', {
      sessionId: SID,
      toolCallId: 'tc-ap-5',
      scope: 'ThisFile',
    });
  });

  it('invokes session_approve_tool with ThisTool from scope menu', () => {
    pushEvent(SID, {
      kind: 'ToolCallApprovalRequested',
      tool_call_id: 'tc-ap-6',
      tool_name: 'fs.edit',
      args_json: FS_EDIT_ARGS,
      preview: PREVIEW,
    });
    const { getByTestId } = render(() => <ChatPane />);
    fireEvent.click(getByTestId('approve-dropdown-btn'));
    fireEvent.click(getByTestId('scope-tool-btn'));
    expect(invokeMock).toHaveBeenCalledWith('session_approve_tool', {
      sessionId: SID,
      toolCallId: 'tc-ap-6',
      scope: 'ThisTool',
    });
  });
});

// ---------------------------------------------------------------------------
// Whitelist auto-approve + pill rendering (F-027)
// ---------------------------------------------------------------------------

describe('Whitelist auto-approve', () => {
  const PREVIEW = { description: 'Edit file /src/foo.ts' };
  const FS_EDIT_ARGS = JSON.stringify({ path: '/src/foo.ts', patch: '...' });

  it('shows whitelisted pill when ThisFile scope was granted for same path', async () => {
    // First render — approve ThisFile scope
    pushEvent(SID, {
      kind: 'ToolCallApprovalRequested',
      tool_call_id: 'tc-wl-1',
      tool_name: 'fs.edit',
      args_json: FS_EDIT_ARGS,
      preview: PREVIEW,
    });
    const { getByTestId, unmount } = render(() => <ChatPane />);
    fireEvent.click(getByTestId('approve-dropdown-btn'));
    fireEvent.click(getByTestId('scope-file-btn'));
    unmount();

    // Reset message store but keep approvals store so whitelist persists
    resetMessagesStore();

    // Second call — same tool + path, should show whitelisted pill
    pushEvent(SID, {
      kind: 'ToolCallApprovalRequested',
      tool_call_id: 'tc-wl-2',
      tool_name: 'fs.edit',
      args_json: FS_EDIT_ARGS,
      preview: PREVIEW,
    });
    const { getAllByTestId } = render(() => <ChatPane />);
    const pills = getAllByTestId('whitelisted-pill');
    expect(pills).toHaveLength(1);
    expect(pills[0]).toHaveTextContent('whitelisted · this file');
  });

  it('auto-invokes session_approve_tool for whitelisted match', async () => {
    // First render — approve ThisTool scope
    pushEvent(SID, {
      kind: 'ToolCallApprovalRequested',
      tool_call_id: 'tc-wl-auto-1',
      tool_name: 'fs.edit',
      args_json: FS_EDIT_ARGS,
      preview: PREVIEW,
    });
    const { getByTestId, unmount } = render(() => <ChatPane />);
    fireEvent.click(getByTestId('approve-dropdown-btn'));
    fireEvent.click(getByTestId('scope-tool-btn'));
    unmount();

    // Reset messages but keep approvals whitelist
    resetMessagesStore();
    invokeMock.mockClear();

    // Second call — same tool, auto-approve effect fires on mount
    pushEvent(SID, {
      kind: 'ToolCallApprovalRequested',
      tool_call_id: 'tc-wl-auto-2',
      tool_name: 'fs.edit',
      args_json: FS_EDIT_ARGS,
      preview: PREVIEW,
    });
    render(() => <ChatPane />);
    expect(invokeMock).toHaveBeenCalledWith('session_approve_tool', expect.objectContaining({
      toolCallId: 'tc-wl-auto-2',
      scope: 'ThisTool',
    }));
  });

  it('hides approval prompt and shows pill when auto-approved', async () => {
    // First render — approve ThisFile scope
    pushEvent(SID, {
      kind: 'ToolCallApprovalRequested',
      tool_call_id: 'tc-wl-hide-1',
      tool_name: 'fs.edit',
      args_json: FS_EDIT_ARGS,
      preview: PREVIEW,
    });
    const { getByTestId, unmount } = render(() => <ChatPane />);
    fireEvent.click(getByTestId('approve-dropdown-btn'));
    fireEvent.click(getByTestId('scope-file-btn'));
    unmount();

    // Reset messages but keep approvals whitelist
    resetMessagesStore();

    // Second call — pill shown, no approval-prompt
    pushEvent(SID, {
      kind: 'ToolCallApprovalRequested',
      tool_call_id: 'tc-wl-hide-2',
      tool_name: 'fs.edit',
      args_json: FS_EDIT_ARGS,
      preview: PREVIEW,
    });
    const { queryByTestId, getAllByTestId } = render(() => <ChatPane />);
    expect(queryByTestId('approval-prompt')).not.toBeInTheDocument();
    const pills = getAllByTestId('whitelisted-pill');
    expect(pills).toHaveLength(1);
  });
});

// ---------------------------------------------------------------------------
// F-079: error handling on invoke rejection
// ---------------------------------------------------------------------------

async function flushMicrotasks(): Promise<void> {
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
}

describe('invoke rejection handling (F-079)', () => {
  it('clears awaitingResponse and surfaces an error turn when session_send_message rejects', async () => {
    invokeMock.mockReset();
    invokeMock.mockRejectedValueOnce(new Error('send failed'));

    const { getByTestId, findByText } = render(() => <ChatPane />);
    const textarea = getByTestId('composer-textarea') as HTMLTextAreaElement;

    fireEvent.input(textarea, { target: { value: 'hello' } });
    fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false, ctrlKey: false, metaKey: false });

    // Composer flips to disabled synchronously when handleSend sets awaitingResponse(true).
    expect(textarea).toBeDisabled();

    // After the rejected invoke promise resolves, awaitingResponse must be cleared
    // (composer re-enabled) AND an error turn must surface the failure.
    await flushMicrotasks();
    expect(textarea).not.toBeDisabled();
    expect(await findByText(/send failed/)).toBeInTheDocument();
  });

  it('surfaces an error turn when session_approve_tool rejects from the inline prompt', async () => {
    invokeMock.mockReset();
    invokeMock.mockRejectedValueOnce(new Error('approve boom'));

    pushEvent(SID, {
      kind: 'ToolCallApprovalRequested',
      tool_call_id: 'tc-rej-approve',
      tool_name: 'fs.edit',
      args_json: JSON.stringify({ path: '/src/x.ts', patch: '...' }),
      preview: { description: 'Edit /src/x.ts' },
    });

    const { getByTestId, findByText } = render(() => <ChatPane />);
    fireEvent.click(getByTestId('approve-once-btn'));

    await flushMicrotasks();
    expect(await findByText(/approve boom/)).toBeInTheDocument();
  });

  it('surfaces an error turn when session_reject_tool rejects', async () => {
    invokeMock.mockReset();
    invokeMock.mockRejectedValueOnce(new Error('reject boom'));

    pushEvent(SID, {
      kind: 'ToolCallApprovalRequested',
      tool_call_id: 'tc-rej-reject',
      tool_name: 'fs.edit',
      args_json: JSON.stringify({ path: '/src/x.ts', patch: '...' }),
      preview: { description: 'Edit /src/x.ts' },
    });

    const { getByTestId, findByText } = render(() => <ChatPane />);
    fireEvent.click(getByTestId('reject-btn'));

    await flushMicrotasks();
    expect(await findByText(/reject boom/)).toBeInTheDocument();
  });
});

// ---------------------------------------------------------------------------
// F-080 item 5: composer message-byte cap (defense-in-depth)
// ---------------------------------------------------------------------------
//
// The Rust side enforces a 128 KiB cap; the composer caps at 100 KiB so the
// user gets immediate feedback without an IPC round trip. Both the warning
// surface and the send-blocking behavior are user-facing contracts.
describe('Composer message-byte cap (F-080)', () => {
  it('does not show the overflow warning under the cap', () => {
    const { getByTestId, queryByTestId } = render(() => <ChatPane />);
    const textarea = getByTestId('composer-textarea') as HTMLTextAreaElement;
    fireEvent.input(textarea, { target: { value: 'hello' } });
    expect(queryByTestId('composer-overflow-warning')).not.toBeInTheDocument();
  });

  it('shows the inline overflow warning when the trimmed payload exceeds the cap', () => {
    const { getByTestId } = render(() => <ChatPane />);
    const textarea = getByTestId('composer-textarea') as HTMLTextAreaElement;
    const overCap = 'a'.repeat(MAX_COMPOSER_BYTES + 1);
    fireEvent.input(textarea, { target: { value: overCap } });
    expect(getByTestId('composer-overflow-warning')).toBeInTheDocument();
  });

  it('blocks send on Enter when over the byte cap', () => {
    invokeMock.mockReset();
    const { getByTestId } = render(() => <ChatPane />);
    const textarea = getByTestId('composer-textarea') as HTMLTextAreaElement;
    const overCap = 'a'.repeat(MAX_COMPOSER_BYTES + 1);
    fireEvent.input(textarea, { target: { value: overCap } });
    fireEvent.keyDown(textarea, {
      key: 'Enter',
      code: 'Enter',
      shiftKey: false,
      ctrlKey: false,
      metaKey: false,
    });
    // No `session_send_message` invocation should have been made — the
    // composer-side cap intercepts the send before it reaches the bridge.
    const sendCalls = invokeMock.mock.calls.filter(
      (c) => c[0] === 'session_send_message',
    );
    expect(sendCalls).toHaveLength(0);
  });

  it('counts UTF-8 bytes, not UTF-16 code units (multi-byte characters)', () => {
    // Each "💥" is 4 UTF-8 bytes but only 2 UTF-16 code units. Picking a count
    // that is over the cap in bytes but well under it in `String.length` units
    // proves the implementation uses TextEncoder rather than `.length`.
    const emoji = '💥';
    const utf8PerChar = new TextEncoder().encode(emoji).length; // 4
    const overCount = Math.ceil(MAX_COMPOSER_BYTES / utf8PerChar) + 1;
    const value = emoji.repeat(overCount);
    // Sanity-check the corner: byte length over cap, code-unit length under.
    expect(new TextEncoder().encode(value).length).toBeGreaterThan(MAX_COMPOSER_BYTES);

    invokeMock.mockReset();
    const { getByTestId } = render(() => <ChatPane />);
    const textarea = getByTestId('composer-textarea') as HTMLTextAreaElement;
    fireEvent.input(textarea, { target: { value } });
    expect(getByTestId('composer-overflow-warning')).toBeInTheDocument();
    fireEvent.keyDown(textarea, {
      key: 'Enter',
      code: 'Enter',
      shiftKey: false,
      ctrlKey: false,
      metaKey: false,
    });
    expect(
      invokeMock.mock.calls.filter((c) => c[0] === 'session_send_message'),
    ).toHaveLength(0);
  });

  it('still sends a payload exactly at the cap', () => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(undefined);
    const { getByTestId } = render(() => <ChatPane />);
    const textarea = getByTestId('composer-textarea') as HTMLTextAreaElement;
    const atCap = 'a'.repeat(MAX_COMPOSER_BYTES);
    fireEvent.input(textarea, { target: { value: atCap } });
    fireEvent.keyDown(textarea, {
      key: 'Enter',
      code: 'Enter',
      shiftKey: false,
      ctrlKey: false,
      metaKey: false,
    });
    expect(invokeMock).toHaveBeenCalledWith(
      'session_send_message',
      expect.objectContaining({ sessionId: SID, text: atCap }),
    );
  });
});

// ---------------------------------------------------------------------------
// F-080 item 6: args_json boundary contract — `fromRustEvent` always
// produces valid JSON, so removing the defensive try/catches in ChatPane
// must not regress the path-extraction or rendering behavior.
// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// F-141: @-trigger + ContextPicker integration in the composer
// ---------------------------------------------------------------------------
//
// The DoD requires: typing `@` opens the picker; selecting a result appends
// a ContextChip to the `ctx-chips` row above the textarea AND removes the
// `@text` span; Escape dismisses without inserting. Full keyboard coverage
// for the picker itself lives in ContextPicker.test.tsx — these tests pin
// the *composer-side* plumbing (trigger detection, chip row population,
// textarea mutation on insert).
describe('Composer @-trigger and ContextPicker integration (F-141)', () => {
  it('does not render the picker while the textarea holds no `@` token', () => {
    const { queryByTestId } = render(() => <ChatPane />);
    expect(queryByTestId('context-picker')).not.toBeInTheDocument();
  });

  it('opens the picker when the user types `@`', () => {
    const { getByTestId, queryByTestId } = render(() => <ChatPane />);
    const textarea = getByTestId('composer-textarea') as HTMLTextAreaElement;
    // Typing `@` at the start of a fresh composer — the simplest trigger.
    textarea.value = '@';
    textarea.selectionStart = 1;
    textarea.selectionEnd = 1;
    fireEvent.input(textarea);
    expect(queryByTestId('context-picker')).toBeInTheDocument();
  });

  it('closes the picker once the user types a space after `@`', () => {
    const { getByTestId, queryByTestId } = render(() => <ChatPane />);
    const textarea = getByTestId('composer-textarea') as HTMLTextAreaElement;
    textarea.value = '@foo';
    textarea.selectionStart = 4;
    textarea.selectionEnd = 4;
    fireEvent.input(textarea);
    expect(queryByTestId('context-picker')).toBeInTheDocument();
    textarea.value = '@foo ';
    textarea.selectionStart = 5;
    textarea.selectionEnd = 5;
    fireEvent.input(textarea);
    expect(queryByTestId('context-picker')).not.toBeInTheDocument();
  });

  it('does not send the message when Enter is pressed while the picker is open', () => {
    invokeMock.mockReset();
    const { getByTestId } = render(() => <ChatPane />);
    const textarea = getByTestId('composer-textarea') as HTMLTextAreaElement;
    textarea.value = '@foo';
    textarea.selectionStart = 4;
    textarea.selectionEnd = 4;
    fireEvent.input(textarea);
    fireEvent.keyDown(textarea, { key: 'Enter' });
    const sendCalls = invokeMock.mock.calls.filter(
      (c) => c[0] === 'session_send_message',
    );
    expect(sendCalls).toHaveLength(0);
  });

  it('renders the ctx-chips row above the textarea', () => {
    const { getByTestId } = render(() => <ChatPane />);
    expect(getByTestId('ctx-chips')).toBeInTheDocument();
  });

  // DoD item 4: "selected result appends a `ContextChip` to `ctx-chips` row;
  // the `@text` span is replaced". `removeAtSpan` is the pure-function half
  // of that (the span removal + caret positioning). Unit-testing it directly
  // decouples the invariant from the picker's async onPick wiring.
  it('removeAtSpan removes an @token at the start of text and keeps the caret at 0', () => {
    const result = removeAtSpan('@foo', 0, 4);
    expect(result.text).toBe('');
    expect(result.caret).toBe(0);
  });

  it('removeAtSpan removes an @token embedded in text and positions the caret at the join', () => {
    const text = 'hello @foo world';
    // caret sits just after "foo" — span is "@foo".
    const result = removeAtSpan(text, 6, 10);
    expect(result.text).toBe('hello  world');
    expect(result.caret).toBe(6);
  });

  it('removeAtSpan preserves trailing text that was after the caret', () => {
    const text = '@partial rest';
    const result = removeAtSpan(text, 0, 8);
    expect(result.text).toBe(' rest');
    expect(result.caret).toBe(0);
  });

  // DoD item 4 end-to-end: render the Composer with a seeded category, open
  // the picker via `@`, pick a result, and assert the chip lands in the
  // ctx-chips row AND the `@text` span is removed from the textarea.
  it('end-to-end: @-trigger → pick result → chip appears in ctx-chips, @text span removed', () => {
    const items = {
      file: [
        { category: 'file' as const, label: 'alpha.ts', value: 'src/alpha.ts' },
      ],
    };
    const { getByTestId, queryByTestId } = render(() => (
      <Composer disabled={false} onSend={() => {}} items={items} />
    ));
    const textarea = getByTestId('composer-textarea') as HTMLTextAreaElement;
    textarea.value = 'pref @foo';
    textarea.selectionStart = 9;
    textarea.selectionEnd = 9;
    fireEvent.input(textarea);
    // Picker is open with the seeded file result.
    expect(queryByTestId('context-picker')).toBeInTheDocument();
    expect(queryByTestId('context-picker-result-0')).toBeInTheDocument();
    // Click the first result.
    fireEvent.mouseDown(getByTestId('context-picker-result-0'));
    // Chip landed in the ctx-chips row with the picked label.
    const chip = getByTestId('ctx-chip');
    expect(chip).toHaveTextContent('alpha.ts');
    // Picker closed.
    expect(queryByTestId('context-picker')).not.toBeInTheDocument();
    // `@text` span removed from the textarea.
    expect(textarea.value).toBe('pref ');
  });

  it('Escape while picker is open dismisses without inserting a chip', () => {
    const { getByTestId, queryByTestId } = render(() => <ChatPane />);
    const textarea = getByTestId('composer-textarea') as HTMLTextAreaElement;
    textarea.value = '@foo';
    textarea.selectionStart = 4;
    textarea.selectionEnd = 4;
    fireEvent.input(textarea);
    expect(queryByTestId('context-picker')).toBeInTheDocument();
    fireEvent.keyDown(window, { key: 'Escape' });
    // Picker must close, no chip inserted (chips row stays empty).
    expect(queryByTestId('context-picker')).not.toBeInTheDocument();
    expect(queryByTestId('ctx-chip')).not.toBeInTheDocument();
  });

  // F-142 DoD item 4: ChatPane wires the resolver registry + provider adapter
  // into the send path — a file chip emits a prepended `<context ...>` block
  // above the user text on the IPC call. Injects a stub registry so the test
  // doesn't need a running Rust backend.
  it('F-142 integration: chip at send time prepends an Anthropic-shaped block to text', async () => {
    const stubRegistry = {
      file: {
        list: async () => [
          { category: 'file' as const, label: 'app.ts', value: '/ws/app.ts' },
        ],
        resolve: async () => ({
          type: 'file' as const,
          path: '/ws/app.ts',
          content: 'body-of-app',
        }),
      },
    };
    const { getByTestId } = render(() => (
      <ChatPane registry={stubRegistry} providerId={'anthropic'} />
    ));
    const textarea = getByTestId('composer-textarea') as HTMLTextAreaElement;
    // Open the picker, pick the only file candidate, type a message, send.
    textarea.value = '@app';
    textarea.selectionStart = 4;
    textarea.selectionEnd = 4;
    fireEvent.input(textarea);
    // `list(query)` runs async; wait a tick for the picker items to populate.
    await new Promise((r) => setTimeout(r, 0));
    fireEvent.mouseDown(getByTestId('context-picker-result-0'));
    fireEvent.input(textarea, { target: { value: 'explain' } });
    fireEvent.keyDown(textarea, { key: 'Enter' });
    // resolveChips is async — wait a microtask for the prefix path.
    await new Promise((r) => setTimeout(r, 0));
    const call = invokeMock.mock.calls.find(
      (c) => c[0] === 'session_send_message',
    );
    expect(call).toBeDefined();
    const sentText = (call![1] as { text: string }).text;
    expect(sentText).toContain('<context type="file" path="/ws/app.ts">');
    expect(sentText).toContain('body-of-app');
    expect(sentText).toContain('explain');
    // The user text follows the context block, separated by a blank line.
    expect(sentText.trim().endsWith('explain')).toBe(true);
  });

  // F-142 DoD item 2: send flow routes chips through the resolver registry
  // and prepends a provider-shaped block to the user text. Uses a Composer
  // directly with a stub `onSend` so the assertion stays scoped to the
  // composer's chip-forwarding contract.
  it('F-142: onSend receives the currently-attached chips and clears them', () => {
    const items = {
      file: [
        { category: 'file' as const, label: 'app.ts', value: '/ws/app.ts' },
      ],
    };
    const onSend = vi.fn();
    const { getByTestId, queryByTestId } = render(() => (
      <Composer disabled={false} onSend={onSend} items={items} />
    ));
    const textarea = getByTestId('composer-textarea') as HTMLTextAreaElement;
    textarea.value = '@app';
    textarea.selectionStart = 4;
    textarea.selectionEnd = 4;
    fireEvent.input(textarea);
    fireEvent.mouseDown(getByTestId('context-picker-result-0'));
    // One chip is now attached.
    expect(getByTestId('ctx-chip')).toHaveTextContent('app.ts');
    // Type a message and send with Enter.
    fireEvent.input(textarea, { target: { value: 'please review' } });
    fireEvent.keyDown(textarea, { key: 'Enter' });
    expect(onSend).toHaveBeenCalledWith(
      'please review',
      expect.arrayContaining([
        expect.objectContaining({
          category: 'file',
          label: 'app.ts',
          value: '/ws/app.ts',
        }),
      ]),
    );
    // Chips are consumed on send.
    expect(queryByTestId('ctx-chip')).not.toBeInTheDocument();
  });
});

describe('args_json boundary contract (F-080)', () => {
  it('renders the path and uses it for whitelist matching when args is a path-bearing object', () => {
    pushEvent(SID, {
      kind: 'ToolCallStarted',
      tool_call_id: 'tc-bc-path',
      tool_name: 'fs.read',
      args_json: JSON.stringify({ path: '/etc/hosts' }),
    });
    const { getByTestId } = render(() => <ChatPane />);
    expect(getByTestId('tool-call-card-tc-bc-path')).toHaveTextContent('/etc/hosts');
  });

  it('falls back to the stringified payload for non-object args (boundary edge: "null")', () => {
    // `fromRustEvent` writes `JSON.stringify(ev['args'] ?? null)` so an absent
    // `args` field arrives here as the literal string "null". The summary
    // should still render that — no try/catch to swallow it.
    pushEvent(SID, {
      kind: 'ToolCallStarted',
      tool_call_id: 'tc-bc-null',
      tool_name: 'noargs',
      args_json: 'null',
    });
    const { getByTestId } = render(() => <ChatPane />);
    expect(getByTestId('tool-call-card-tc-bc-null')).toHaveTextContent('null');
  });
});

// ---------------------------------------------------------------------------
// F-036: persistent approvals — approve/revoke at workspace/user level
// ---------------------------------------------------------------------------

import { setActiveWorkspaceRoot } from '../../stores/session';
import {
  seedPersistentApprovals,
  getApprovalWhitelist,
} from '../../stores/approvals';

describe('ChatPane — persistent approvals (F-036)', () => {
  const PREVIEW = { description: 'Edit file /src/foo.ts' };
  const FS_EDIT_ARGS = JSON.stringify({ path: '/src/foo.ts', patch: '...' });

  beforeEach(() => {
    setActiveWorkspaceRoot('/ws');
  });

  afterEach(() => {
    setActiveWorkspaceRoot(null);
  });

  it('invokes save_approval with level=workspace for ThisFile + Workspace tier', () => {
    pushEvent(SID, {
      kind: 'ToolCallApprovalRequested',
      tool_call_id: 'tc-f36-ws',
      tool_name: 'fs.edit',
      args_json: FS_EDIT_ARGS,
      preview: PREVIEW,
    });
    const { getByTestId } = render(() => <ChatPane />);
    fireEvent.click(getByTestId('level-workspace-btn'));
    fireEvent.click(getByTestId('approve-dropdown-btn'));
    fireEvent.click(getByTestId('scope-file-btn'));

    const saveCall = invokeMock.mock.calls.find((c) => c[0] === 'save_approval');
    expect(saveCall).toBeDefined();
    expect(saveCall?.[1]).toMatchObject({
      level: 'workspace',
      workspaceRoot: '/ws',
      entry: expect.objectContaining({
        scope_key: 'file:fs.edit:/src/foo.ts',
        tool_name: 'fs.edit',
        label: 'this file',
      }),
    });
  });

  it('invokes save_approval with level=user for ThisTool + User tier', () => {
    pushEvent(SID, {
      kind: 'ToolCallApprovalRequested',
      tool_call_id: 'tc-f36-user',
      tool_name: 'fs.edit',
      args_json: FS_EDIT_ARGS,
      preview: PREVIEW,
    });
    const { getByTestId } = render(() => <ChatPane />);
    fireEvent.click(getByTestId('level-user-btn'));
    fireEvent.click(getByTestId('approve-dropdown-btn'));
    fireEvent.click(getByTestId('scope-tool-btn'));

    const saveCall = invokeMock.mock.calls.find((c) => c[0] === 'save_approval');
    expect(saveCall?.[1]).toMatchObject({
      level: 'user',
      workspaceRoot: '/ws',
      entry: expect.objectContaining({
        scope_key: 'tool:fs.edit',
        label: 'this tool',
      }),
    });
  });

  it('does NOT invoke save_approval for Session level', () => {
    pushEvent(SID, {
      kind: 'ToolCallApprovalRequested',
      tool_call_id: 'tc-f36-session',
      tool_name: 'fs.edit',
      args_json: FS_EDIT_ARGS,
      preview: PREVIEW,
    });
    const { getByTestId } = render(() => <ChatPane />);
    fireEvent.click(getByTestId('approve-dropdown-btn'));
    fireEvent.click(getByTestId('scope-file-btn'));

    expect(
      invokeMock.mock.calls.filter((c) => c[0] === 'save_approval'),
    ).toHaveLength(0);
  });

  it('does NOT invoke save_approval for a one-shot Once approval even with Workspace selected', () => {
    pushEvent(SID, {
      kind: 'ToolCallApprovalRequested',
      tool_call_id: 'tc-f36-once-ws',
      tool_name: 'fs.edit',
      args_json: FS_EDIT_ARGS,
      preview: PREVIEW,
    });
    const { getByTestId } = render(() => <ChatPane />);
    fireEvent.click(getByTestId('level-workspace-btn'));
    fireEvent.click(getByTestId('approve-once-btn'));

    expect(
      invokeMock.mock.calls.filter((c) => c[0] === 'save_approval'),
    ).toHaveLength(0);
  });

  it('renders workspace-level pill with provenance suffix when seeded', () => {
    seedPersistentApprovals(SID, [
      {
        scope_key: 'file:fs.edit:/src/foo.ts',
        tool_name: 'fs.edit',
        label: 'this file',
        level: 'workspace',
      },
    ]);
    pushEvent(SID, {
      kind: 'ToolCallApprovalRequested',
      tool_call_id: 'tc-f36-pill-ws',
      tool_name: 'fs.edit',
      args_json: FS_EDIT_ARGS,
      preview: PREVIEW,
    });
    const { getByTestId } = render(() => <ChatPane />);
    expect(getByTestId('whitelisted-pill')).toHaveTextContent(
      'whitelisted · this file · workspace',
    );
  });

  it('renders user-level pill with provenance suffix when seeded', () => {
    seedPersistentApprovals(SID, [
      {
        scope_key: 'tool:fs.edit',
        tool_name: 'fs.edit',
        label: 'this tool',
        level: 'user',
      },
    ]);
    pushEvent(SID, {
      kind: 'ToolCallApprovalRequested',
      tool_call_id: 'tc-f36-pill-user',
      tool_name: 'fs.edit',
      args_json: FS_EDIT_ARGS,
      preview: PREVIEW,
    });
    const { getByTestId } = render(() => <ChatPane />);
    expect(getByTestId('whitelisted-pill')).toHaveTextContent(
      'whitelisted · this tool · user',
    );
  });

  it('invokes remove_approval when a workspace-level entry is revoked via the pill', () => {
    seedPersistentApprovals(SID, [
      {
        scope_key: 'tool:fs.edit',
        tool_name: 'fs.edit',
        label: 'this tool',
        level: 'workspace',
      },
    ]);
    pushEvent(SID, {
      kind: 'ToolCallApprovalRequested',
      tool_call_id: 'tc-f36-revoke-ws',
      tool_name: 'fs.edit',
      args_json: FS_EDIT_ARGS,
      preview: PREVIEW,
    });
    invokeMock.mockClear();
    const { getByTestId } = render(() => <ChatPane />);
    fireEvent.click(getByTestId('whitelisted-pill'));
    fireEvent.click(getByTestId('revoke-btn'));

    const removeCall = invokeMock.mock.calls.find((c) => c[0] === 'remove_approval');
    expect(removeCall?.[1]).toEqual({
      scopeKey: 'tool:fs.edit',
      level: 'workspace',
      workspaceRoot: '/ws',
    });
    // The entry is also removed from the in-memory whitelist.
    expect('tool:fs.edit' in getApprovalWhitelist(SID).entries).toBe(false);
  });

  it('does NOT invoke remove_approval when a session-level entry is revoked', () => {
    pushEvent(SID, {
      kind: 'ToolCallApprovalRequested',
      tool_call_id: 'tc-f36-revoke-session-1',
      tool_name: 'fs.edit',
      args_json: FS_EDIT_ARGS,
      preview: PREVIEW,
    });
    const { getByTestId } = render(() => <ChatPane />);
    // Approve ThisFile at session level first.
    fireEvent.click(getByTestId('approve-dropdown-btn'));
    fireEvent.click(getByTestId('scope-file-btn'));

    // A second call with the same key — pill rendered, revoke.
    resetMessagesStore();
    invokeMock.mockClear();
    pushEvent(SID, {
      kind: 'ToolCallApprovalRequested',
      tool_call_id: 'tc-f36-revoke-session-2',
      tool_name: 'fs.edit',
      args_json: FS_EDIT_ARGS,
      preview: PREVIEW,
    });
    const second = render(() => <ChatPane />);
    fireEvent.click(second.getByTestId('whitelisted-pill'));
    fireEvent.click(second.getByTestId('revoke-btn'));

    expect(
      invokeMock.mock.calls.filter((c) => c[0] === 'remove_approval'),
    ).toHaveLength(0);
  });
});
