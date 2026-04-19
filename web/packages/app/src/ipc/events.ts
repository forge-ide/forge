// Rust → TS event adapter (F-037).
//
// `forged` emits `forge_core::Event` with `#[serde(tag="type", rename_all="snake_case")]`,
// so payloads arrive at the webview as `{"type":"user_message", id, at, text, …}`.
// The messages store discriminates on a different shape (`kind: 'UserMessage'`,
// `message_id` instead of `id`, `args_json` string instead of `args` value, etc.).
// This adapter is the single conversion point — call it at the IPC boundary.
// Returns `null` for variants that have no renderable effect.
//
// F-064 / M12 / T7 — Runtime narrowing. Every required field is checked with
// `typeof` / shape predicates before the event crosses into the store. If a
// malformed payload arrives (daemon bug, version skew, compromised bridge
// writer), we drop the event (return `null`) rather than `as string`-casting
// `undefined`/`number`/object values into fields downstream code assumes are
// strings. A single `warn` per (type, missing field) keys the drift so it
// surfaces in the console without flooding.

import type { SessionEvent } from '../stores/messages';

// ---------------------------------------------------------------------------
// Narrowing helpers
// ---------------------------------------------------------------------------

function isString(v: unknown): v is string {
  return typeof v === 'string';
}

function isObjectWith<K extends string>(
  v: unknown,
  key: K,
): v is Record<K, unknown> {
  return typeof v === 'object' && v !== null && key in v;
}

// Warn once per (type, reason) so a malformed payload surfaces but doesn't
// spam the console when the daemon is emitting the same bad shape repeatedly.
const warnedDrops = new Set<string>();
function warnDrop(type: string, reason: string): void {
  const key = `${type}:${reason}`;
  if (warnedDrops.has(key)) return;
  warnedDrops.add(key);
  // eslint-disable-next-line no-console
  console.warn(
    `[ipc/events] dropped malformed ${type} event: ${reason}`,
  );
}

// ---------------------------------------------------------------------------
// Adapter
// ---------------------------------------------------------------------------

export function fromRustEvent(rustEvent: unknown): SessionEvent | null {
  if (typeof rustEvent !== 'object' || rustEvent === null) return null;
  const ev = rustEvent as Record<string, unknown>;
  const type = ev['type'];

  if (type === 'user_message') {
    const id = ev['id'];
    const text = ev['text'];
    if (!isString(id)) {
      warnDrop('user_message', 'id missing or not a string');
      return null;
    }
    if (!isString(text)) {
      warnDrop('user_message', 'text missing or not a string');
      return null;
    }
    return { kind: 'UserMessage', message_id: id, text };
  }

  if (type === 'tool_call_rejected') {
    const id = ev['id'];
    if (!isString(id)) {
      warnDrop('tool_call_rejected', 'id missing or not a string');
      return null;
    }
    const reason = ev['reason'];
    return {
      kind: 'ToolCallFailed',
      tool_call_id: id,
      error: isString(reason) && reason.length > 0 ? reason : 'rejected',
    };
  }

  if (type === 'tool_call_completed') {
    const id = ev['id'];
    if (!isString(id)) {
      warnDrop('tool_call_completed', 'id missing or not a string');
      return null;
    }
    return {
      kind: 'ToolCallCompleted',
      tool_call_id: id,
      result_summary: JSON.stringify(ev['result'] ?? null).slice(0, 200),
    };
  }

  if (type === 'tool_call_approval_requested') {
    const id = ev['id'];
    if (!isString(id)) {
      warnDrop('tool_call_approval_requested', 'id missing or not a string');
      return null;
    }
    const preview = ev['preview'];
    if (!isObjectWith(preview, 'description') || !isString(preview.description)) {
      warnDrop(
        'tool_call_approval_requested',
        'preview missing or preview.description not a string',
      );
      return null;
    }
    return {
      kind: 'ToolCallApprovalRequested',
      tool_call_id: id,
      preview: { description: preview.description },
    };
  }

  if (type === 'tool_call_started') {
    const id = ev['id'];
    if (!isString(id)) {
      warnDrop('tool_call_started', 'id missing or not a string');
      return null;
    }
    const tool = ev['tool'];
    if (!isString(tool)) {
      warnDrop('tool_call_started', 'tool missing or not a string');
      return null;
    }
    const parallelGroup = ev['parallel_group'];
    const out: SessionEvent = {
      kind: 'ToolCallStarted',
      tool_call_id: id,
      tool_name: tool,
      args_json: JSON.stringify(ev['args'] ?? null),
    };
    if (typeof parallelGroup === 'number') {
      out.batch_id = String(parallelGroup);
    }
    return out;
  }

  if (type === 'assistant_delta') {
    const id = ev['id'];
    if (!isString(id)) {
      warnDrop('assistant_delta', 'id missing or not a string');
      return null;
    }
    const delta = ev['delta'];
    if (!isString(delta)) {
      warnDrop('assistant_delta', 'delta missing or not a string');
      return null;
    }
    return { kind: 'AssistantDelta', message_id: id, delta };
  }

  if (type === 'assistant_message') {
    // The orchestrator emits AssistantMessage twice per turn:
    //   1. at stream-open: stream_finalised: false, text: ""
    //   2. at stream-close: stream_finalised: true, text: <full>
    // Only the second drives a store transition — the first would push an
    // empty, non-streaming assistant turn that intercepts subsequent deltas
    // and suppresses the streaming cursor. Drop stream-open.
    if (ev['stream_finalised'] !== true) return null;
    const id = ev['id'];
    if (!isString(id)) {
      warnDrop('assistant_message', 'id missing or not a string');
      return null;
    }
    const text = ev['text'];
    if (!isString(text)) {
      warnDrop('assistant_message', 'text missing or not a string');
      return null;
    }
    return { kind: 'AssistantMessage', message_id: id, text };
  }

  return null;
}
