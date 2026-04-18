import { beforeEach, describe, expect, it, vi } from 'vitest';

const { invokeMock, listenMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  listenMock: vi.fn(),
}));

vi.mock('@tauri-apps/api/core', () => ({
  invoke: invokeMock,
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: listenMock,
}));

import {
  sessionHello,
  sessionSubscribe,
  sessionSendMessage,
  sessionApproveTool,
  sessionRejectTool,
  onSessionEvent,
  SESSION_EVENT,
} from './session';

describe('session ipc wrappers', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    listenMock.mockReset();
  });

  it('sessionHello invokes `session_hello` with sessionId + socketPath', async () => {
    invokeMock.mockResolvedValue({
      session_id: 'abc',
      workspace: '/ws',
      started_at: '2026-04-15T14:22:00Z',
      event_seq: 0,
      schema_version: 1,
    });

    const ack = await sessionHello('abc', '/tmp/forge.sock');

    expect(invokeMock).toHaveBeenCalledWith('session_hello', {
      sessionId: 'abc',
      socketPath: '/tmp/forge.sock',
    });
    expect(ack.session_id).toBe('abc');
  });

  it('sessionHello omits socketPath when unspecified', async () => {
    invokeMock.mockResolvedValue({
      session_id: 'abc',
      workspace: '',
      started_at: '',
      event_seq: 0,
      schema_version: 1,
    });

    await sessionHello('abc');

    expect(invokeMock).toHaveBeenCalledWith('session_hello', {
      sessionId: 'abc',
      socketPath: undefined,
    });
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
