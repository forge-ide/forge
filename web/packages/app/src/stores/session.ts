import { createSignal } from 'solid-js';
import { createStore } from 'solid-js/store';
import type { SessionId, SessionState } from '@forge/ipc';

export interface SessionSummary {
  id: SessionId;
  state: SessionState;
}

export const [activeSessionId, setActiveSessionId] = createSignal<SessionId | null>(null);

export const [sessions, setSessions] = createStore<Record<SessionId, SessionSummary>>({});
