// Adapter tests — fromRustEvent() maps Rust-serialized forge_core::Event
// shapes (`#[serde(tag="type", rename_all="snake_case")]`) into the TS
// messages store's SessionEvent union. See F-037 / #73.
//
// Wire shapes used as inputs here are pinned by
// `crates/forge-core/tests/event_wire_shape.rs`. That file is the "comparable
// harness" called out in F-037's DoD: each Event variant is constructed as a
// real Rust value and round-tripped through `serde_json::to_value`, with the
// resulting JSON asserted against a hard-coded expected shape. The same
// expected shapes are the inputs below. When the Rust enum changes, that
// Rust test fails first; update the failing cases, then mirror the new
// shapes into this file.

import { describe, it, expect } from 'vitest';
import { fromRustEvent } from './events';

describe('fromRustEvent — user_message', () => {
  it('renames id → message_id and strips non-rendering fields', () => {
    const rust = {
      type: 'user_message',
      id: 'mid-1',
      at: '2026-04-18T10:00:00Z',
      text: 'hello',
      context: [],
      branch_parent: null,
    };

    expect(fromRustEvent(rust)).toEqual({
      kind: 'UserMessage',
      message_id: 'mid-1',
      text: 'hello',
    });
  });
});

describe('fromRustEvent — non-rendering variants return null', () => {
  // The enum-value strings below (SessionPersistence::Persist → "Persist",
  // RosterScope::SessionWide → "SessionWide", EndReason::UserExit → "UserExit",
  // etc.) reflect serde's PascalCase default — forge_core/types.rs and
  // event.rs carry no `#[serde(rename_all)]` on these enums.
  const nullCases = [
    {
      type: 'session_started',
      at: '2026-04-18T10:00:00Z',
      workspace: '/w',
      agent: null,
      persistence: 'Persist',
    },
    { type: 'session_ended', at: '2026-04-18T10:00:00Z', reason: 'UserExit', archived: true },
    { type: 'branch_selected', parent: 'a', selected: 'b' },
    { type: 'sub_agent_spawned', parent: 'p', child: 'c', from_msg: 'm' },
    { type: 'background_agent_started', id: 'ba-1', agent: 'a', at: '2026-04-18T10:00:00Z' },
    { type: 'background_agent_completed', id: 'ba-1', at: '2026-04-18T10:00:00Z' },
    {
      type: 'usage_tick',
      provider: 'p',
      model: 'm',
      tokens_in: 1,
      tokens_out: 2,
      cost_usd: 0,
      scope: 'SessionWide',
    },
    {
      type: 'context_compacted',
      at: '2026-04-18T10:00:00Z',
      summarized_turns: 3,
      summary_msg_id: 's',
      trigger: 'AutoAt98Pct',
    },
    {
      type: 'tool_call_approved',
      id: 'tc-x',
      by: 'User',
      scope: 'Once',
      at: '2026-04-18T10:00:00Z',
    },
  ];

  it.each(nullCases)('returns null for type=$type', (ev) => {
    expect(fromRustEvent(ev)).toBeNull();
  });
});

describe('fromRustEvent — invalid input returns null (never throws)', () => {
  it.each([
    ['null', null],
    ['undefined', undefined],
    ['string', 'not an event'],
    ['number', 42],
    ['array', [1, 2, 3]],
    ['empty object', {}],
    ['unknown type', { type: 'something_we_do_not_know' }],
    ['missing type', { id: 'x', text: 'y' }],
  ] as Array<[string, unknown]>)('returns null for %s', (_label, input) => {
    expect(fromRustEvent(input)).toBeNull();
  });
});

describe('fromRustEvent — tool_call_rejected', () => {
  it('maps to ToolCallFailed with reason as error', () => {
    const rust = {
      type: 'tool_call_rejected',
      id: 'tc-rej-1',
      reason: 'user denied',
    };

    expect(fromRustEvent(rust)).toEqual({
      kind: 'ToolCallFailed',
      tool_call_id: 'tc-rej-1',
      error: 'user denied',
    });
  });

  it('falls back to "rejected" when reason is null', () => {
    const rust = {
      type: 'tool_call_rejected',
      id: 'tc-rej-2',
      reason: null,
    };

    expect(fromRustEvent(rust)).toEqual({
      kind: 'ToolCallFailed',
      tool_call_id: 'tc-rej-2',
      error: 'rejected',
    });
  });
});

describe('fromRustEvent — tool_call_completed', () => {
  it('stringifies result into result_summary and renames id → tool_call_id', () => {
    const rust = {
      type: 'tool_call_completed',
      id: 'tc-7',
      result: { ok: true, bytes: 42 },
      duration_ms: 12,
      at: '2026-04-18T10:00:05Z',
    };

    expect(fromRustEvent(rust)).toEqual({
      kind: 'ToolCallCompleted',
      tool_call_id: 'tc-7',
      result_summary: '{"ok":true,"bytes":42}',
    });
  });

  it('truncates long result_summary at 200 chars', () => {
    const big = 'x'.repeat(500);
    const rust = {
      type: 'tool_call_completed',
      id: 'tc-8',
      result: { content: big },
      duration_ms: 1,
      at: '2026-04-18T10:00:05Z',
    };

    const out = fromRustEvent(rust);
    expect(out).toMatchObject({ kind: 'ToolCallCompleted', tool_call_id: 'tc-8' });
    expect((out as { result_summary: string }).result_summary.length).toBe(200);
  });
});

describe('fromRustEvent — tool_call_approval_requested', () => {
  it('passes through preview and renames id → tool_call_id (Rust carries no tool name/args here)', () => {
    const rust = {
      type: 'tool_call_approval_requested',
      id: 'tc-9',
      preview: { description: 'Edit file /src/foo.ts' },
    };

    expect(fromRustEvent(rust)).toEqual({
      kind: 'ToolCallApprovalRequested',
      tool_call_id: 'tc-9',
      preview: { description: 'Edit file /src/foo.ts' },
    });
  });
});

describe('fromRustEvent — tool_call_started', () => {
  it('renames id → tool_call_id, tool → tool_name, stringifies args as args_json', () => {
    const rust = {
      type: 'tool_call_started',
      id: 'tc-1',
      msg: 'mid-3',
      tool: 'fs.read',
      args: { path: 'readable.txt' },
      at: '2026-04-18T10:00:03Z',
      parallel_group: null,
    };

    expect(fromRustEvent(rust)).toEqual({
      kind: 'ToolCallStarted',
      tool_call_id: 'tc-1',
      tool_name: 'fs.read',
      args_json: '{"path":"readable.txt"}',
    });
  });

  it('stringifies parallel_group u32 into batch_id when present', () => {
    const rust = {
      type: 'tool_call_started',
      id: 'tc-2',
      msg: 'mid-3',
      tool: 'fs.read',
      args: { path: 'a.txt' },
      at: '2026-04-18T10:00:03Z',
      parallel_group: 7,
    };

    expect(fromRustEvent(rust)).toEqual({
      kind: 'ToolCallStarted',
      tool_call_id: 'tc-2',
      tool_name: 'fs.read',
      args_json: '{"path":"a.txt"}',
      batch_id: '7',
    });
  });
});

describe('fromRustEvent — assistant_delta', () => {
  it('maps delta id → message_id and preserves delta text', () => {
    const rust = {
      type: 'assistant_delta',
      id: 'mid-3',
      at: '2026-04-18T10:00:02Z',
      delta: 'partial ',
    };

    expect(fromRustEvent(rust)).toEqual({
      kind: 'AssistantDelta',
      message_id: 'mid-3',
      delta: 'partial ',
    });
  });
});

describe('fromRustEvent — assistant_message', () => {
  it('maps the finalized stream form (stream_finalised: true) to AssistantMessage', () => {
    const rust = {
      type: 'assistant_message',
      id: 'mid-2',
      provider: 'mock',
      model: 'mock-1',
      at: '2026-04-18T10:00:01Z',
      stream_finalised: true,
      text: 'hi there',
      branch_parent: null,
      branch_variant_index: 0,
    };

    expect(fromRustEvent(rust)).toEqual({
      kind: 'AssistantMessage',
      message_id: 'mid-2',
      text: 'hi there',
    });
  });

  it('returns null for the stream-open sentinel (stream_finalised: false)', () => {
    // Rust emits an empty AssistantMessage with stream_finalised:false at stream
    // start (orchestrator.rs:118). Forwarding it would push an empty non-streaming
    // turn into the store, and the first AssistantDelta would find that turn and
    // append to it without ever setting streamingMessageId — breaking the cursor.
    // The first delta must be the one that creates the streaming turn.
    const rust = {
      type: 'assistant_message',
      id: 'mid-open',
      provider: 'mock',
      model: 'mock-1',
      at: '2026-04-18T10:00:00Z',
      stream_finalised: false,
      text: '',
      branch_parent: null,
      branch_variant_index: 0,
    };

    expect(fromRustEvent(rust)).toBeNull();
  });
});
