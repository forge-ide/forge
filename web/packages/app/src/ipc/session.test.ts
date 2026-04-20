import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const { listenMock } = vi.hoisted(() => ({
  listenMock: vi.fn(),
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: listenMock,
}));

import { setInvokeForTesting } from '../lib/tauri';
import {
  sessionHello,
  sessionSubscribe,
  sessionSendMessage,
  sessionApproveTool,
  sessionRejectTool,
  onSessionEvent,
  SESSION_EVENT,
  getPersistentApprovals,
  saveApproval,
  removeApproval,
} from './session';

describe('session ipc wrappers', () => {
  let invokeMock: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    invokeMock = vi.fn();
    setInvokeForTesting(invokeMock as never);
    listenMock.mockReset();
  });

  afterEach(() => {
    setInvokeForTesting(null);
  });

  it('sessionHello invokes `session_hello` with sessionId only', async () => {
    invokeMock.mockResolvedValue({
      session_id: 'abc',
      workspace: '/ws',
      started_at: '2026-04-15T14:22:00Z',
      event_seq: 0,
      schema_version: 1,
    });

    const ack = await sessionHello('abc');

    // F-052: no `socketPath` in the payload — the shell always resolves
    // the UDS via `default_socket_path(session_id)`.
    expect(invokeMock).toHaveBeenCalledWith('session_hello', {
      sessionId: 'abc',
    });
    expect(ack.session_id).toBe('abc');
  });

  it('sessionSubscribe invokes `session_subscribe` with since', async () => {
    invokeMock.mockResolvedValue(undefined);

    await sessionSubscribe('abc', 42);

    expect(invokeMock).toHaveBeenCalledWith('session_subscribe', {
      sessionId: 'abc',
      since: 42,
    });
  });

  it('sessionSendMessage invokes `session_send_message`', async () => {
    invokeMock.mockResolvedValue(undefined);

    await sessionSendMessage('abc', 'hello');

    expect(invokeMock).toHaveBeenCalledWith('session_send_message', {
      sessionId: 'abc',
      text: 'hello',
    });
  });

  it('sessionApproveTool invokes `session_approve_tool` with scope', async () => {
    invokeMock.mockResolvedValue(undefined);

    await sessionApproveTool('abc', 'call-1', 'Once');

    expect(invokeMock).toHaveBeenCalledWith('session_approve_tool', {
      sessionId: 'abc',
      toolCallId: 'call-1',
      scope: 'Once',
    });
  });

  it('sessionRejectTool invokes `session_reject_tool` with optional reason', async () => {
    invokeMock.mockResolvedValue(undefined);

    await sessionRejectTool('abc', 'call-1');
    expect(invokeMock).toHaveBeenLastCalledWith('session_reject_tool', {
      sessionId: 'abc',
      toolCallId: 'call-1',
      reason: undefined,
    });

    await sessionRejectTool('abc', 'call-2', 'nope');
    expect(invokeMock).toHaveBeenLastCalledWith('session_reject_tool', {
      sessionId: 'abc',
      toolCallId: 'call-2',
      reason: 'nope',
    });
  });

  it('onSessionEvent subscribes to session:event with callback receiving payload', async () => {
    const unlisten = vi.fn();
    listenMock.mockResolvedValue(unlisten);

    const handler = vi.fn();
    const off = await onSessionEvent(handler);

    expect(listenMock).toHaveBeenCalledWith(SESSION_EVENT, expect.any(Function));
    expect(SESSION_EVENT).toBe('session:event');

    // Simulate Tauri emitting an event by calling the bound listener.
    const listener = listenMock.mock.calls[0]![1] as (
      event: { payload: unknown },
    ) => void;
    const payload = { session_id: 'abc', seq: 1, event: { kind: 'AssistantDelta' } };
    listener({ payload });

    expect(handler).toHaveBeenCalledWith(payload);
    expect(off).toBe(unlisten);
  });
});

// F-073: the test seam contract — `setInvokeForTesting()` must intercept
// commands routed through the typed helpers in `ipc/session.ts`. This guards
// against regressions where a helper bypasses the wrapper by importing
// `invoke` directly from the underlying Tauri API.
describe('setInvokeForTesting intercepts ipc/session commands', () => {
  afterEach(() => {
    setInvokeForTesting(null);
  });

  it('routes sessionHello through the wrapper seam', async () => {
    const spy = vi.fn().mockResolvedValue({
      session_id: 'seam',
      workspace: '/ws',
      started_at: '2026-04-19T00:00:00Z',
      event_seq: 0,
      schema_version: 1,
    });
    setInvokeForTesting(spy as never);

    await sessionHello('seam');

    expect(spy).toHaveBeenCalledTimes(1);
    expect(spy).toHaveBeenCalledWith('session_hello', { sessionId: 'seam' });
  });

  it('routes sessionSubscribe through the wrapper seam', async () => {
    const spy = vi.fn().mockResolvedValue(undefined);
    setInvokeForTesting(spy as never);

    await sessionSubscribe('seam', 7);

    expect(spy).toHaveBeenCalledWith('session_subscribe', {
      sessionId: 'seam',
      since: 7,
    });
  });

  it('routes sessionSendMessage through the wrapper seam', async () => {
    const spy = vi.fn().mockResolvedValue(undefined);
    setInvokeForTesting(spy as never);

    await sessionSendMessage('seam', 'hi');

    expect(spy).toHaveBeenCalledWith('session_send_message', {
      sessionId: 'seam',
      text: 'hi',
    });
  });

  it('routes sessionApproveTool through the wrapper seam', async () => {
    const spy = vi.fn().mockResolvedValue(undefined);
    setInvokeForTesting(spy as never);

    await sessionApproveTool('seam', 'tc-1', 'ThisFile');

    expect(spy).toHaveBeenCalledWith('session_approve_tool', {
      sessionId: 'seam',
      toolCallId: 'tc-1',
      scope: 'ThisFile',
    });
  });

  it('routes sessionRejectTool through the wrapper seam', async () => {
    const spy = vi.fn().mockResolvedValue(undefined);
    setInvokeForTesting(spy as never);

    await sessionRejectTool('seam', 'tc-2', 'no');

    expect(spy).toHaveBeenCalledWith('session_reject_tool', {
      sessionId: 'seam',
      toolCallId: 'tc-2',
      reason: 'no',
    });
  });
});

// ---------------------------------------------------------------------------
// F-036: persistent approval wrappers
// ---------------------------------------------------------------------------

describe('persistent approval ipc wrappers (F-036)', () => {
  let invokeMock: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    invokeMock = vi.fn();
    setInvokeForTesting(invokeMock as never);
  });

  afterEach(() => {
    setInvokeForTesting(null);
  });

  it('getPersistentApprovals invokes `get_persistent_approvals` with workspaceRoot', async () => {
    invokeMock.mockResolvedValue([
      {
        scope_key: 'tool:fs.write',
        tool_name: 'fs.write',
        label: 'this tool',
        level: 'workspace',
      },
    ]);

    const entries = await getPersistentApprovals('/ws');

    expect(invokeMock).toHaveBeenCalledWith('get_persistent_approvals', {
      workspaceRoot: '/ws',
    });
    expect(entries).toEqual([
      {
        scope_key: 'tool:fs.write',
        tool_name: 'fs.write',
        label: 'this tool',
        level: 'workspace',
      },
    ]);
  });

  it('saveApproval invokes `save_approval` with entry, level, workspaceRoot', async () => {
    invokeMock.mockResolvedValue(undefined);

    await saveApproval(
      { scope_key: 'tool:fs.edit', tool_name: 'fs.edit', label: 'this tool' },
      'user',
      '/ws',
    );

    expect(invokeMock).toHaveBeenCalledWith('save_approval', {
      entry: { scope_key: 'tool:fs.edit', tool_name: 'fs.edit', label: 'this tool' },
      level: 'user',
      workspaceRoot: '/ws',
    });
  });

  it('removeApproval invokes `remove_approval` with scopeKey, level, workspaceRoot', async () => {
    invokeMock.mockResolvedValue(undefined);

    await removeApproval('tool:shell.exec', 'workspace', '/repo');

    expect(invokeMock).toHaveBeenCalledWith('remove_approval', {
      scopeKey: 'tool:shell.exec',
      level: 'workspace',
      workspaceRoot: '/repo',
    });
  });
});
