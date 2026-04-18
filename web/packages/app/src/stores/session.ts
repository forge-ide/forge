import { createSignal } from 'solid-js';
import { createStore } from 'solid-js/store';
import type { SessionId, SessionState } from '@forge/ipc';
import {
  onSessionEvent,
  type SessionEventPayload,
} from '../ipc/session';
import type { UnlistenFn } from '@tauri-apps/api/event';

export interface SessionSummary {
  id: SessionId;
  state: SessionState;
}

export const [activeSessionId, setActiveSessionId] = createSignal<SessionId | null>(null);

export const [sessions, setSessions] = createStore<Record<SessionId, SessionSummary>>({});

/**
 * Per-session view of the most recent event received from the shell bridge.
 * Components that need fine-grained state (messages, tool calls, etc.) will
 * derive from this or dispatch to richer stores. For F-020 the pump's job is
 * simply to prove event delivery works end-to-end.
 */
export interface SessionEventSlot {
  lastSeq: number;
  lastEvent: unknown;
}

export const [sessionEvents, setSessionEvents] = createStore<
  Record<SessionId, SessionEventSlot>
>({});

/**
 * Subscribe the store to the `session:event` Tauri event. Returns the
 * unlisten handle so the caller (app bootstrap) can tear it down on unmount.
 */
export async function initSessionEventPump(): Promise<UnlistenFn> {
  return onSessionEvent((payload: SessionEventPayload) => {
    setSessionEvents(payload.session_id, {
      lastSeq: payload.seq,
      lastEvent: payload.event,
    });
  });
}

/** Test helper — clears the event slots store between tests. */
export function resetSessionEventStore(): void {
  setSessionEvents({});
}
