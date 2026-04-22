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
    const result = ev['result'];
    // F-447: the Rust wire `result` is `{ ok: bool, preview?: string, error?: string }`.
    // Phase 2 collapsed result into a single `result_summary` stringified blob;
    // Phase 3 needs the structured pieces for the expanded body's "preview",
    // status glyph, and duration readout. Keep `result_summary` for backwards
    // compatibility with existing consumers — the store carries both.
    const out: SessionEvent = {
      kind: 'ToolCallCompleted',
      tool_call_id: id,
      result_summary: JSON.stringify(result ?? null).slice(0, 200),
    };
    if (isObjectWith(result, 'ok') && typeof result.ok === 'boolean') {
      out.result_ok = result.ok;
    }
    if (isObjectWith(result, 'preview') && isString(result.preview)) {
      out.result_preview = result.preview;
    }
    if (isObjectWith(result, 'error') && isString(result.error)) {
      out.result_error = result.error;
    }
    const durationMs = ev['duration_ms'];
    if (typeof durationMs === 'number' && Number.isFinite(durationMs)) {
      out.duration_ms = durationMs;
    }
    return out;
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

  // F-136: sub-agent spawn — the orchestrator emitted `SubAgentSpawned`
  // from the session's event log. Carries parent/child `AgentInstanceId`s
  // and the originating `MessageId`. Optional `agent_name` is accepted if
  // present so a future shell-side enrichment can surface it without
  // another round-trip; absent today.
  if (type === 'sub_agent_spawned') {
    const parent = ev['parent'];
    const child = ev['child'];
    const fromMsg = ev['from_msg'];
    if (!isString(parent)) {
      warnDrop('sub_agent_spawned', 'parent missing or not a string');
      return null;
    }
    if (!isString(child)) {
      warnDrop('sub_agent_spawned', 'child missing or not a string');
      return null;
    }
    if (!isString(fromMsg)) {
      warnDrop('sub_agent_spawned', 'from_msg missing or not a string');
      return null;
    }
    const agentName = ev['agent_name'];
    // F-448 Phase 3: optional header-chip fields. Non-string model / non-
    // number tool_count is a daemon wire skew — drop the field silently
    // (the banner hides absent chips cleanly) rather than failing the whole
    // event and losing the spawn itself.
    const model = ev['model'];
    const toolCount = ev['tool_count'];
    const out: SessionEvent = {
      kind: 'SubAgentSpawned',
      parent_instance_id: parent,
      child_instance_id: child,
      from_msg: fromMsg,
    };
    if (isString(agentName)) out.agent_name = agentName;
    if (isString(model)) out.model = model;
    if (typeof toolCount === 'number' && Number.isFinite(toolCount)) {
      out.tool_count = toolCount;
    }
    return out;
  }

  // F-136: sub-agent terminal — `BackgroundAgentRegistry` (F-137) forwards
  // this onto the session bus when the child's orchestrator lifecycle hits
  // `Completed` or `Failed`. Flips the matching banner to `done`.
  if (type === 'background_agent_completed') {
    const id = ev['id'];
    if (!isString(id)) {
      warnDrop('background_agent_completed', 'id missing or not a string');
      return null;
    }
    return { kind: 'BackgroundAgentCompleted', instance_id: id };
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
    // F-145: forward branch coordinates + provider/model/at when present.
    // Older fixtures may omit these; the store treats missing values as
    // "root variant, no metadata" — no branch rendering chrome appears.
    const branchParentRaw = ev['branch_parent'];
    const branchParent: string | null =
      branchParentRaw === null || branchParentRaw === undefined
        ? null
        : isString(branchParentRaw)
          ? branchParentRaw
          : null;
    const variantIndexRaw = ev['branch_variant_index'];
    const branchVariantIndex =
      typeof variantIndexRaw === 'number' && Number.isFinite(variantIndexRaw)
        ? variantIndexRaw
        : 0;
    const provider = ev['provider'];
    const model = ev['model'];
    const at = ev['at'];
    const out: SessionEvent = {
      kind: 'AssistantMessage',
      message_id: id,
      text,
      branch_parent: branchParent,
      branch_variant_index: branchVariantIndex,
    };
    if (isString(provider)) out.provider = provider;
    if (isString(model)) out.model = model;
    if (isString(at)) out.at = at;
    return out;
  }

  // F-144: branch selection — flips the active variant of a group.
  if (type === 'branch_selected') {
    const parent = ev['parent'];
    const selected = ev['selected'];
    if (!isString(parent)) {
      warnDrop('branch_selected', 'parent missing or not a string');
      return null;
    }
    if (!isString(selected)) {
      warnDrop('branch_selected', 'selected missing or not a string');
      return null;
    }
    return { kind: 'BranchSelected', parent, selected };
  }

  // F-145: branch deletion — tombstones a variant.
  if (type === 'branch_deleted') {
    const parent = ev['parent'];
    const variantIndex = ev['variant_index'];
    if (!isString(parent)) {
      warnDrop('branch_deleted', 'parent missing or not a string');
      return null;
    }
    if (typeof variantIndex !== 'number' || !Number.isFinite(variantIndex)) {
      warnDrop('branch_deleted', 'variant_index missing or not a finite number');
      return null;
    }
    return { kind: 'BranchDeleted', parent, variant_index: variantIndex };
  }

  return null;
}
