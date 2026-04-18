import { describe, expect, it, vi, beforeEach } from 'vitest';
import { render, fireEvent } from '@solidjs/testing-library';
import type { SessionId } from '@forge/ipc';

// --- Mocks (hoisted so vi.mock works) ---
const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));

vi.mock('../../lib/tauri', () => ({ invoke: invokeMock }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

// --- Store imports (after mocks) ---
import {
  pushEvent,
  setAwaitingResponse,
  resetMessagesStore,
} from '../../stores/messages';
import { setActiveSessionId } from '../../stores/session';
import { resetApprovalsStore } from '../../stores/approvals';
import { ChatPane } from './ChatPane';

const SID = 'session-chat-test' as SessionId;

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(undefined);
  resetMessagesStore();
  resetApprovalsStore();
  setActiveSessionId(SID);
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
