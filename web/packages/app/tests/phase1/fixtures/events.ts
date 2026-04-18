// Event factories matching the wire shape of `forge_core::Event`.
//
// `forge_core::Event` serializes with `#[serde(tag = "type", rename_all =
// "snake_case")]`, so payloads reaching the webview look like
// `{"type":"user_message", id, at, text, …}`. The app's TS/Rust boundary
// adapter (`src/ipc/events.ts`) maps that into the messages store's
// SessionEvent union. These factories produce the wire shape verbatim so the
// Playwright UATs exercise the same adapter the production code uses.
//
// When `crates/forge-core/src/event.rs` adds or changes a variant, mirror it
// here. Keep factories variadic but strict about required fields.

export interface SessionEventPayload {
  session_id: string;
  seq: number;
  event: unknown;
}

let seqCounter = 100;
export function nextSeq(): number {
  return seqCounter++;
}

const ISO_TIME = '2026-04-18T10:00:00Z';

export function userMessage(sessionId: string, text: string): SessionEventPayload {
  return {
    session_id: sessionId,
    seq: nextSeq(),
    event: {
      type: 'user_message',
      id: `u-${nextSeq()}`,
      at: ISO_TIME,
      text,
      context: [],
      branch_parent: null,
    },
  };
}

export function assistantDelta(
  sessionId: string,
  messageId: string,
  delta: string,
): SessionEventPayload {
  return {
    session_id: sessionId,
    seq: nextSeq(),
    event: {
      type: 'assistant_delta',
      id: messageId,
      at: ISO_TIME,
      delta,
    },
  };
}

export function assistantMessageFinal(
  sessionId: string,
  messageId: string,
  text: string,
): SessionEventPayload {
  return {
    session_id: sessionId,
    seq: nextSeq(),
    event: {
      type: 'assistant_message',
      id: messageId,
      provider: 'mock',
      model: 'mock-1',
      at: ISO_TIME,
      stream_finalised: true,
      text,
      branch_parent: null,
      branch_variant_index: 0,
    },
  };
}

export function toolCallStarted(
  sessionId: string,
  toolCallId: string,
  tool: string,
  args: Record<string, unknown>,
  parallelGroup?: number,
): SessionEventPayload {
  return {
    session_id: sessionId,
    seq: nextSeq(),
    event: {
      type: 'tool_call_started',
      id: toolCallId,
      msg: 'mid-test',
      tool,
      args,
      at: ISO_TIME,
      parallel_group: parallelGroup ?? null,
    },
  };
}

export function toolCallApprovalRequested(
  sessionId: string,
  toolCallId: string,
  preview: { description: string },
): SessionEventPayload {
  return {
    session_id: sessionId,
    seq: nextSeq(),
    event: {
      type: 'tool_call_approval_requested',
      id: toolCallId,
      preview,
    },
  };
}

export function toolCallCompleted(
  sessionId: string,
  toolCallId: string,
  result: { ok: boolean; preview?: string; error?: string },
): SessionEventPayload {
  return {
    session_id: sessionId,
    seq: nextSeq(),
    event: {
      type: 'tool_call_completed',
      id: toolCallId,
      result,
      duration_ms: 1,
      at: ISO_TIME,
    },
  };
}
