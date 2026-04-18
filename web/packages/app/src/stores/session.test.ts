import { describe, expect, it } from 'vitest';
import type { SessionId } from '@forge/ipc';
import {
  activeSessionId,
  setActiveSessionId,
  sessions,
  setSessions,
} from './session';

describe('session store', () => {
  it('activeSessionId is null by default', () => {
    expect(activeSessionId()).toBeNull();
  });

  it('setActiveSessionId updates the signal', () => {
    const id = 'session-1' as SessionId;
    setActiveSessionId(id);
    expect(activeSessionId()).toBe(id);
    setActiveSessionId(null);
    expect(activeSessionId()).toBeNull();
  });

  it('sessions store is initially empty', () => {
    expect(Object.keys(sessions)).toHaveLength(0);
  });

  it('setSessions can seed an entry keyed by SessionId', () => {
    const id = 'session-2' as SessionId;
    setSessions(id, { id, state: 'Active' });
    expect(sessions[id]).toEqual({ id, state: 'Active' });
  });
});
