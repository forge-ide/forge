import { beforeEach, describe, expect, it } from 'vitest';
import {
  pushEvent,
  setAwaitingResponse,
  getMessagesState,
  resetMessagesStore,
} from './messages';
import type { SessionId } from '@forge/ipc';

const SID = 'session-test' as SessionId;

describe('messages store', () => {
  beforeEach(() => {
    resetMessagesStore();
  });

  describe('UserMessage events', () => {
    it('appends a user turn', () => {
      pushEvent(SID, { kind: 'UserMessage', text: 'hello', message_id: 'msg-1' });
      const state = getMessagesState(SID);
      expect(state.turns).toHaveLength(1);
      expect(state.turns[0]).toMatchObject({ type: 'user', text: 'hello', message_id: 'msg-1' });
    });
  });

  describe('AssistantMessage events (complete)', () => {
    it('appends a non-streaming assistant turn', () => {
      pushEvent(SID, { kind: 'AssistantMessage', text: 'hi there', message_id: 'msg-2' });
      const state = getMessagesState(SID);
      expect(state.turns).toHaveLength(1);
      expect(state.turns[0]).toMatchObject({
        type: 'assistant',
        text: 'hi there',
        message_id: 'msg-2',
        isStreaming: false,
      });
    });

    it('clears streamingMessageId on AssistantMessage', () => {
      pushEvent(SID, { kind: 'AssistantDelta', delta: 'partial', message_id: 'msg-3' });
      pushEvent(SID, { kind: 'AssistantMessage', text: 'partial complete', message_id: 'msg-3' });
      const state = getMessagesState(SID);
      expect(state.streamingMessageId).toBeNull();
    });

    it('clears awaitingResponse on AssistantMessage', () => {
      setAwaitingResponse(SID, true);
      pushEvent(SID, { kind: 'AssistantMessage', text: 'response', message_id: 'msg-4' });
      expect(getMessagesState(SID).awaitingResponse).toBe(false);
    });
  });

  describe('AssistantDelta events (streaming)', () => {
    it('creates a streaming assistant turn on first delta', () => {
      pushEvent(SID, { kind: 'AssistantDelta', delta: 'Hello', message_id: 'msg-5' });
      const state = getMessagesState(SID);
      expect(state.turns).toHaveLength(1);
      expect(state.turns[0]).toMatchObject({
        type: 'assistant',
        isStreaming: true,
        message_id: 'msg-5',
      });
      expect(state.streamingMessageId).toBe('msg-5');
    });

    it('accumulates delta text across multiple deltas', () => {
      pushEvent(SID, { kind: 'AssistantDelta', delta: 'Hello', message_id: 'msg-6' });
      pushEvent(SID, { kind: 'AssistantDelta', delta: ' world', message_id: 'msg-6' });
      pushEvent(SID, { kind: 'AssistantDelta', delta: '!', message_id: 'msg-6' });
      const state = getMessagesState(SID);
      expect(state.turns[0]).toMatchObject({ text: 'Hello world!' });
    });

    it('clears awaitingResponse when first delta arrives', () => {
      setAwaitingResponse(SID, true);
      pushEvent(SID, { kind: 'AssistantDelta', delta: 'start', message_id: 'msg-7' });
      expect(getMessagesState(SID).awaitingResponse).toBe(false);
    });

    it('does not create duplicate turns for the same message_id', () => {
      pushEvent(SID, { kind: 'AssistantDelta', delta: 'chunk1', message_id: 'msg-8' });
      pushEvent(SID, { kind: 'AssistantDelta', delta: 'chunk2', message_id: 'msg-8' });
      expect(getMessagesState(SID).turns).toHaveLength(1);
    });
  });

  describe('ToolCallStarted / ToolCallCompleted events', () => {
    it('appends a tool placeholder with status in-progress on ToolCallStarted', () => {
      pushEvent(SID, {
        kind: 'ToolCallStarted',
        tool_call_id: 'tc-1',
        tool_name: 'fs.read',
        args_json: '{"path":"/foo"}',
      });
      const state = getMessagesState(SID);
      expect(state.turns).toHaveLength(1);
      expect(state.turns[0]).toMatchObject({
        type: 'tool_placeholder',
        tool_call_id: 'tc-1',
        tool_name: 'fs.read',
        status: 'in-progress',
      });
    });

    it('preserves args_json on the turn', () => {
      pushEvent(SID, {
        kind: 'ToolCallStarted',
        tool_call_id: 'tc-1',
        tool_name: 'fs.read',
        args_json: '{"path":"/foo"}',
      });
      const state = getMessagesState(SID);
      expect(state.turns[0]).toMatchObject({
        type: 'tool_placeholder',
        args_json: '{"path":"/foo"}',
      });
    });

    it('preserves batch_id on the turn when provided', () => {
      pushEvent(SID, {
        kind: 'ToolCallStarted',
        tool_call_id: 'tc-1',
        tool_name: 'fs.read',
        args_json: '{}',
        batch_id: 'batch-42',
      });
      const state = getMessagesState(SID);
      expect(state.turns[0]).toMatchObject({
        type: 'tool_placeholder',
        batch_id: 'batch-42',
      });
    });

    it('records started_at as a number on ToolCallStarted', () => {
      const before = Date.now();
      pushEvent(SID, {
        kind: 'ToolCallStarted',
        tool_call_id: 'tc-1',
        tool_name: 'fs.read',
        args_json: '{}',
      });
      const after = Date.now();
      const state = getMessagesState(SID);
      const turn = state.turns[0] as { started_at: number };
      expect(typeof turn.started_at).toBe('number');
      expect(turn.started_at).toBeGreaterThanOrEqual(before);
      expect(turn.started_at).toBeLessThanOrEqual(after);
    });

    it('marks status completed and stores result_summary on ToolCallCompleted', () => {
      pushEvent(SID, {
        kind: 'ToolCallStarted',
        tool_call_id: 'tc-2',
        tool_name: 'fs.write',
        args_json: '{}',
      });
      pushEvent(SID, {
        kind: 'ToolCallCompleted',
        tool_call_id: 'tc-2',
        result_summary: 'wrote 42 bytes',
      });
      const state = getMessagesState(SID);
      expect(state.turns[0]).toMatchObject({
        type: 'tool_placeholder',
        status: 'completed',
        result_summary: 'wrote 42 bytes',
      });
    });

    it('computes a non-negative duration_ms on ToolCallCompleted', () => {
      pushEvent(SID, {
        kind: 'ToolCallStarted',
        tool_call_id: 'tc-2',
        tool_name: 'fs.write',
        args_json: '{}',
      });
      pushEvent(SID, {
        kind: 'ToolCallCompleted',
        tool_call_id: 'tc-2',
        result_summary: 'ok',
      });
      const state = getMessagesState(SID);
      const turn = state.turns[0] as { duration_ms?: number };
      expect(typeof turn.duration_ms).toBe('number');
      expect(turn.duration_ms).toBeGreaterThanOrEqual(0);
    });

    it('sets status errored and stores error on ToolCallFailed', () => {
      pushEvent(SID, {
        kind: 'ToolCallStarted',
        tool_call_id: 'tc-3',
        tool_name: 'shell.exec',
        args_json: '{"cmd":"rm -rf /"}',
      });
      pushEvent(SID, {
        kind: 'ToolCallFailed',
        tool_call_id: 'tc-3',
        error: 'permission denied',
      });
      const state = getMessagesState(SID);
      expect(state.turns[0]).toMatchObject({
        type: 'tool_placeholder',
        status: 'errored',
        error: 'permission denied',
      });
    });

    it('computes a non-negative duration_ms on ToolCallFailed', () => {
      pushEvent(SID, {
        kind: 'ToolCallStarted',
        tool_call_id: 'tc-3',
        tool_name: 'shell.exec',
        args_json: '{}',
      });
      pushEvent(SID, {
        kind: 'ToolCallFailed',
        tool_call_id: 'tc-3',
        error: 'timeout',
      });
      const state = getMessagesState(SID);
      const turn = state.turns[0] as { duration_ms?: number };
      expect(typeof turn.duration_ms).toBe('number');
      expect(turn.duration_ms).toBeGreaterThanOrEqual(0);
    });
  });

  describe('ToolCallApprovalRequested events', () => {
    it('transitions an existing placeholder to awaiting-approval', () => {
      pushEvent(SID, {
        kind: 'ToolCallStarted',
        tool_call_id: 'tc-approval',
        tool_name: 'fs.edit',
        args_json: '{"path":"/src/foo.ts"}',
      });
      pushEvent(SID, {
        kind: 'ToolCallApprovalRequested',
        tool_call_id: 'tc-approval',
        tool_name: 'fs.edit',
        args_json: '{"path":"/src/foo.ts"}',
        preview: { description: 'Edit file /src/foo.ts: 2 hunks' },
      });
      const state = getMessagesState(SID);
      expect(state.turns).toHaveLength(1);
      expect(state.turns[0]).toMatchObject({
        type: 'tool_placeholder',
        tool_call_id: 'tc-approval',
        status: 'awaiting-approval',
        preview: { description: 'Edit file /src/foo.ts: 2 hunks' },
      });
    });

    it('creates a fresh placeholder when no prior ToolCallStarted', () => {
      pushEvent(SID, {
        kind: 'ToolCallApprovalRequested',
        tool_call_id: 'tc-fresh',
        tool_name: 'fs.write',
        args_json: '{"path":"/src/bar.ts"}',
        preview: { description: 'Write file /src/bar.ts' },
      });
      const state = getMessagesState(SID);
      expect(state.turns).toHaveLength(1);
      expect(state.turns[0]).toMatchObject({
        type: 'tool_placeholder',
        tool_call_id: 'tc-fresh',
        tool_name: 'fs.write',
        status: 'awaiting-approval',
        preview: { description: 'Write file /src/bar.ts' },
      });
    });

    it('attaches the preview description to the turn', () => {
      pushEvent(SID, {
        kind: 'ToolCallApprovalRequested',
        tool_call_id: 'tc-preview',
        tool_name: 'shell.exec',
        args_json: '{}',
        preview: { description: 'Run: /bin/sh -c echo hi (cwd /workspace)' },
      });
      const state = getMessagesState(SID);
      const turn = state.turns[0] as { preview?: { description: string } };
      expect(turn.preview?.description).toBe('Run: /bin/sh -c echo hi (cwd /workspace)');
    });

    it('does not duplicate turns if both ToolCallStarted and ApprovalRequested arrive', () => {
      pushEvent(SID, {
        kind: 'ToolCallStarted',
        tool_call_id: 'tc-dup',
        tool_name: 'fs.edit',
        args_json: '{}',
      });
      pushEvent(SID, {
        kind: 'ToolCallApprovalRequested',
        tool_call_id: 'tc-dup',
        tool_name: 'fs.edit',
        args_json: '{}',
        preview: { description: 'Edit' },
      });
      const state = getMessagesState(SID);
      expect(state.turns).toHaveLength(1);
    });
  });

  describe('Error events', () => {
    it('appends an error turn', () => {
      pushEvent(SID, { kind: 'Error', message: 'ECONNREFUSED 127.0.0.1:11434' });
      const state = getMessagesState(SID);
      expect(state.turns).toHaveLength(1);
      expect(state.turns[0]).toMatchObject({
        type: 'error',
        message: 'ECONNREFUSED 127.0.0.1:11434',
      });
    });

    it('clears awaitingResponse on Error', () => {
      setAwaitingResponse(SID, true);
      pushEvent(SID, { kind: 'Error', message: 'failed' });
      expect(getMessagesState(SID).awaitingResponse).toBe(false);
    });

    it('clears streamingMessageId on Error', () => {
      pushEvent(SID, { kind: 'AssistantDelta', delta: 'partial', message_id: 'msg-9' });
      pushEvent(SID, { kind: 'Error', message: 'stream cut' });
      expect(getMessagesState(SID).streamingMessageId).toBeNull();
    });
  });

  describe('awaitingResponse', () => {
    it('defaults to false', () => {
      expect(getMessagesState(SID).awaitingResponse).toBe(false);
    });

    it('can be set to true', () => {
      setAwaitingResponse(SID, true);
      expect(getMessagesState(SID).awaitingResponse).toBe(true);
    });
  });

  describe('multi-session isolation', () => {
    it('keeps turns isolated per session', () => {
      const SID2 = 'session-other' as SessionId;
      pushEvent(SID, { kind: 'UserMessage', text: 'hello sid1', message_id: 'a' });
      pushEvent(SID2, { kind: 'UserMessage', text: 'hello sid2', message_id: 'b' });
      expect(getMessagesState(SID).turns).toHaveLength(1);
      expect(getMessagesState(SID2).turns).toHaveLength(1);
      expect(getMessagesState(SID).turns[0]!.type).toBe('user');
    });
  });
});
