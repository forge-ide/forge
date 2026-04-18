import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { SessionId, ToolCallId, ApprovalScope } from '@forge/ipc';

export const SESSION_EVENT = 'session:event';

/** Mirror of `forge_ipc::HelloAck`. */
export interface HelloAck {
  session_id: string;
  workspace: string;
  started_at: string;
  event_seq: number;
  schema_version: number;
}

/** Payload emitted by the shell's bridge reader task. */
export interface SessionEventPayload {
  session_id: SessionId;
  seq: number;
  event: unknown;
}

export async function sessionHello(
  sessionId: SessionId,
  socketPath?: string,
): Promise<HelloAck> {
  return invoke<HelloAck>('session_hello', {
    sessionId,
    socketPath,
  });
}

export async function sessionSubscribe(
  sessionId: SessionId,
  since = 0,
): Promise<void> {
  await invoke('session_subscribe', { sessionId, since });
}

export async function sessionSendMessage(
  sessionId: SessionId,
  text: string,
): Promise<void> {
  await invoke('session_send_message', { sessionId, text });
}

export async function sessionApproveTool(
  sessionId: SessionId,
  toolCallId: ToolCallId,
  scope: ApprovalScope,
): Promise<void> {
  await invoke('session_approve_tool', { sessionId, toolCallId, scope });
}

export async function sessionRejectTool(
  sessionId: SessionId,
  toolCallId: ToolCallId,
  reason?: string,
): Promise<void> {
  await invoke('session_reject_tool', { sessionId, toolCallId, reason });
}

/**
 * Subscribe to session events from the shell. Returns an unlisten handle.
 * The callback receives the payload unwrapped from Tauri's event envelope.
 */
export async function onSessionEvent(
  handler: (payload: SessionEventPayload) => void,
): Promise<UnlistenFn> {
  return listen<SessionEventPayload>(SESSION_EVENT, (event) => {
    handler(event.payload);
  });
}
