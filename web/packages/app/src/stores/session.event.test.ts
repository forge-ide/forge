import { beforeEach, describe, expect, it, vi } from 'vitest';

const { listenMock, unlistenMock } = vi.hoisted(() => ({
  listenMock: vi.fn(),
  unlistenMock: vi.fn(),
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: listenMock,
}));
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}));

import type { SessionId } from '@forge/ipc';
import {
  sessionEvents,
  initSessionEventPump,
  resetSessionEventStore,
} from './session';

describe('session event pump', () => {
  beforeEach(() => {
    listenMock.mockReset();
    unlistenMock.mockReset();
    resetSessionEventStore();
  });

  it('records the latest seq and event payload per session', async () => {
    listenMock.mockResolvedValue(unlistenMock);

    const off = await initSessionEventPump();
    expect(listenMock).toHaveBeenCalledWith(
      'session:event',
      expect.any(Function),
    );

    const listener = listenMock.mock.calls[0]![1] as (event: {
      payload: unknown;
    }) => void;

    const sid = 'session-1' as SessionId;
    listener({
      payload: {
        session_id: sid,
        seq: 1,
        event: { kind: 'UserMessage', text: 'hi' },
      },
    });
    listener({
      payload: {
        session_id: sid,
        seq: 2,
        event: { kind: 'AssistantDelta', delta: 'yo' },
      },
    });

    const slot = sessionEvents[sid];
    expect(slot).toBeDefined();
    expect(slot!.lastSeq).toBe(2);
    expect(slot!.lastEvent).toEqual({
      kind: 'AssistantDelta',
      delta: 'yo',
    });

    // Handle returned by initSessionEventPump is the unlisten callback.
    expect(off).toBe(unlistenMock);
  });
});
