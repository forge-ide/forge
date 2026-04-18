// Rust → TS event adapter (F-037).
//
// `forged` emits `forge_core::Event` with `#[serde(tag="type", rename_all="snake_case")]`,
// so payloads arrive at the webview as `{"type":"user_message", id, at, text, …}`.
// The messages store discriminates on a different shape (`kind: 'UserMessage'`,
// `message_id` instead of `id`, `args_json` string instead of `args` value, etc.).
// This adapter is the single conversion point — call it at the IPC boundary.
// Returns `null` for variants that have no renderable effect.

import type { SessionEvent } from '../stores/messages';

export function fromRustEvent(rustEvent: unknown): SessionEvent | null {
  if (typeof rustEvent !== 'object' || rustEvent === null) return null;
  const ev = rustEvent as Record<string, unknown>;
  const type = ev['type'];

  if (type === 'user_message') {
    return {
      kind: 'UserMessage',
      message_id: ev['id'] as string,
      text: ev['text'] as string,
    };
  }

  if (type === 'tool_call_rejected') {
    const reason = ev['reason'];
    return {
      kind: 'ToolCallFailed',
      tool_call_id: ev['id'] as string,
      error: typeof reason === 'string' && reason.length > 0 ? reason : 'rejected',
    };
  }

  if (type === 'tool_call_completed') {
    return {
      kind: 'ToolCallCompleted',
      tool_call_id: ev['id'] as string,
      result_summary: JSON.stringify(ev['result'] ?? null).slice(0, 200),
    };
  }

  if (type === 'tool_call_approval_requested') {
    return {
      kind: 'ToolCallApprovalRequested',
      tool_call_id: ev['id'] as string,
      preview: ev['preview'] as { description: string },
    };
  }

  if (type === 'tool_call_started') {
    const parallelGroup = ev['parallel_group'];
    const out: SessionEvent = {
      kind: 'ToolCallStarted',
      tool_call_id: ev['id'] as string,
      tool_name: ev['tool'] as string,
      args_json: JSON.stringify(ev['args'] ?? null),
    };
    if (typeof parallelGroup === 'number') {
      out.batch_id = String(parallelGroup);
    }
    return out;
  }

  if (type === 'assistant_delta') {
    return {
      kind: 'AssistantDelta',
      message_id: ev['id'] as string,
      delta: ev['delta'] as string,
    };
  }

  if (type === 'assistant_message') {
    // The orchestrator emits AssistantMessage twice per turn:
    //   1. at stream-open: stream_finalised: false, text: ""
    //   2. at stream-close: stream_finalised: true, text: <full>
    // Only the second drives a store transition — the first would push an
    // empty, non-streaming assistant turn that intercepts subsequent deltas
    // and suppresses the streaming cursor. Drop stream-open.
    if (ev['stream_finalised'] !== true) return null;
    return {
      kind: 'AssistantMessage',
      message_id: ev['id'] as string,
      text: ev['text'] as string,
    };
  }

  return null;
}
