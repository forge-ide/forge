import { createStore, produce, reconcile } from 'solid-js/store';
import type { SessionId } from '@forge/ipc';

// ---------------------------------------------------------------------------
// Event shapes arriving from the IPC bridge (session:event payload)
// ---------------------------------------------------------------------------

/** Preview data from the shell for a pending tool call approval. */
export interface ApprovalPreview {
  /** Human-readable description of what the tool call will do. */
  description: string;
}

export type SessionEvent =
  | { kind: 'UserMessage'; text: string; message_id: string }
  | { kind: 'AssistantMessage'; text: string; message_id: string }
  | { kind: 'AssistantDelta'; delta: string; message_id: string }
  | { kind: 'ToolCallStarted'; tool_call_id: string; tool_name: string; args_json: string; batch_id?: string }
  // tool_name/args_json are optional — the Rust wire event carries only id+preview,
  // and the approval always follows a ToolCallStarted, so the store normally
  // transitions an existing placeholder. They remain for the fallback branch
  // (placeholder missing) used by the pre-wire unit tests.
  | { kind: 'ToolCallApprovalRequested'; tool_call_id: string; tool_name?: string; args_json?: string; preview: ApprovalPreview }
  | { kind: 'ToolCallCompleted'; tool_call_id: string; result_summary: string }
  | { kind: 'ToolCallFailed'; tool_call_id: string; error: string }
  | { kind: 'Error'; message: string }
  | { kind: 'StreamingStarted' }
  | { kind: 'StreamingStopped' };

// ---------------------------------------------------------------------------
// Chat turn shapes (derived, used for rendering)
// ---------------------------------------------------------------------------

export type ToolCallStatus = 'in-progress' | 'awaiting-approval' | 'completed' | 'errored';

export type ChatTurn =
  | { type: 'user'; text: string; message_id: string }
  | { type: 'assistant'; text: string; message_id: string; isStreaming: boolean }
  | {
      type: 'tool_placeholder';
      tool_call_id: string;
      tool_name: string;
      args_json: string;
      batch_id?: string;
      status: ToolCallStatus;
      started_at: number;
      duration_ms?: number;
      result_summary?: string;
      error?: string;
      /** Populated when status is 'awaiting-approval'. */
      preview?: ApprovalPreview;
    }
  | { type: 'error'; message: string };

// ---------------------------------------------------------------------------
// Per-session messages state
// ---------------------------------------------------------------------------

export interface MessagesState {
  turns: ChatTurn[];
  awaitingResponse: boolean;
  streamingMessageId: string | null;
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

const [messagesStore, setMessagesStore] = createStore<Record<string, MessagesState>>({});

function ensureSession(sessionId: SessionId): void {
  if (!messagesStore[sessionId]) {
    setMessagesStore(sessionId, {
      turns: [],
      awaitingResponse: false,
      streamingMessageId: null,
    });
  }
}

export function getMessagesState(sessionId: SessionId): MessagesState {
  ensureSession(sessionId);
  return messagesStore[sessionId]!;
}

export function setAwaitingResponse(sessionId: SessionId, value: boolean): void {
  ensureSession(sessionId);
  setMessagesStore(sessionId, 'awaitingResponse', value);
}

export function pushEvent(sessionId: SessionId, event: SessionEvent): void {
  ensureSession(sessionId);

  switch (event.kind) {
    case 'UserMessage': {
      setMessagesStore(
        produce((s) => {
          s[sessionId]!.turns.push({
            type: 'user',
            text: event.text,
            message_id: event.message_id,
          });
        }),
      );
      break;
    }

    case 'AssistantMessage': {
      setMessagesStore(
        produce((s) => {
          const state = s[sessionId]!;
          // If there's a streaming turn for this message_id, update it in place.
          const idx = state.turns.findIndex(
            (t) => t.type === 'assistant' && t.message_id === event.message_id,
          );
          if (idx >= 0) {
            const turn = state.turns[idx] as { type: 'assistant'; text: string; message_id: string; isStreaming: boolean };
            turn.text = event.text;
            turn.isStreaming = false;
          } else {
            state.turns.push({
              type: 'assistant',
              text: event.text,
              message_id: event.message_id,
              isStreaming: false,
            });
          }
          state.streamingMessageId = null;
          state.awaitingResponse = false;
        }),
      );
      break;
    }

    case 'AssistantDelta': {
      setMessagesStore(
        produce((s) => {
          const state = s[sessionId]!;
          state.awaitingResponse = false;
          const idx = state.turns.findIndex(
            (t) => t.type === 'assistant' && t.message_id === event.message_id,
          );
          if (idx >= 0) {
            const turn = state.turns[idx] as { type: 'assistant'; text: string; message_id: string; isStreaming: boolean };
            turn.text += event.delta;
          } else {
            state.turns.push({
              type: 'assistant',
              text: event.delta,
              message_id: event.message_id,
              isStreaming: true,
            });
            state.streamingMessageId = event.message_id;
          }
        }),
      );
      break;
    }

    case 'ToolCallStarted': {
      setMessagesStore(
        produce((s) => {
          s[sessionId]!.turns.push({
            type: 'tool_placeholder',
            tool_call_id: event.tool_call_id,
            tool_name: event.tool_name,
            args_json: event.args_json,
            ...(event.batch_id !== undefined ? { batch_id: event.batch_id } : {}),
            status: 'in-progress',
            started_at: Date.now(),
          });
        }),
      );
      break;
    }

    case 'ToolCallApprovalRequested': {
      setMessagesStore(
        produce((s) => {
          const state = s[sessionId]!;
          // If there's already a tool_placeholder for this call (from ToolCallStarted),
          // transition it to awaiting-approval and attach the preview.
          const idx = state.turns.findIndex(
            (t) => t.type === 'tool_placeholder' && t.tool_call_id === event.tool_call_id,
          );
          if (idx >= 0) {
            const turn = state.turns[idx] as Extract<ChatTurn, { type: 'tool_placeholder' }>;
            turn.status = 'awaiting-approval';
            turn.preview = event.preview;
          } else {
            // No prior ToolCallStarted — push a fresh placeholder. In the
            // Rust wire path this branch is unreachable (approval always
            // follows a started event), so tool_name/args_json fall back to
            // safe defaults when the event omits them.
            state.turns.push({
              type: 'tool_placeholder',
              tool_call_id: event.tool_call_id,
              tool_name: event.tool_name ?? 'unknown',
              args_json: event.args_json ?? '{}',
              status: 'awaiting-approval',
              started_at: Date.now(),
              preview: event.preview,
            });
          }
        }),
      );
      break;
    }

    case 'ToolCallCompleted': {
      setMessagesStore(
        produce((s) => {
          const state = s[sessionId]!;
          const idx = state.turns.findIndex(
            (t) => t.type === 'tool_placeholder' && t.tool_call_id === event.tool_call_id,
          );
          if (idx >= 0) {
            const turn = state.turns[idx] as Extract<ChatTurn, { type: 'tool_placeholder' }>;
            turn.status = 'completed';
            turn.result_summary = event.result_summary;
            turn.duration_ms = Date.now() - turn.started_at;
          }
        }),
      );
      break;
    }

    case 'ToolCallFailed': {
      setMessagesStore(
        produce((s) => {
          const state = s[sessionId]!;
          const idx = state.turns.findIndex(
            (t) => t.type === 'tool_placeholder' && t.tool_call_id === event.tool_call_id,
          );
          if (idx >= 0) {
            const turn = state.turns[idx] as Extract<ChatTurn, { type: 'tool_placeholder' }>;
            turn.status = 'errored';
            turn.error = event.error;
            turn.duration_ms = Date.now() - turn.started_at;
          }
        }),
      );
      break;
    }

    case 'Error': {
      setMessagesStore(
        produce((s) => {
          const state = s[sessionId]!;
          state.turns.push({ type: 'error', message: event.message });
          state.awaitingResponse = false;
          state.streamingMessageId = null;
        }),
      );
      break;
    }

    case 'StreamingStarted':
    case 'StreamingStopped':
      // Handled implicitly via delta/message events.
      break;
  }
}

/** Test helper — clears all message state between tests. */
export function resetMessagesStore(): void {
  setMessagesStore(reconcile({}));
}
