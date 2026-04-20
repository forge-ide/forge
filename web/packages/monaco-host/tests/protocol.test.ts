// F-121 smoke test. Exercises the postMessage protocol with a stub editor.
// Does NOT import monaco-editor or monaco-languageclient — those bundles
// are not jsdom-safe. See README "Test harness".

import { describe, expect, it, vi } from 'vitest';
import {
  createIframeProtocol,
  type EditorInboundMessage,
  type EditorLike,
  type EditorOutboundMessage,
  type MessageListener,
} from '../src/protocol.js';

/** Recording stub editor. Satisfies the `EditorLike` surface. */
function makeStubEditor(): EditorLike & {
  setValueCalls: string[];
  focusCalls: number;
  emitChange: (value: string) => void;
} {
  let buffer = '';
  const listeners: Array<(v: string) => void> = [];
  const stub = {
    setValueCalls: [] as string[],
    focusCalls: 0,
    setValue(value: string) {
      buffer = value;
      stub.setValueCalls.push(value);
    },
    getValue() {
      return buffer;
    },
    focus() {
      stub.focusCalls += 1;
    },
    onDidChangeContent(cb: (v: string) => void) {
      listeners.push(cb);
      return { dispose: () => void 0 };
    },
    dispose() {},
    emitChange(value: string) {
      buffer = value;
      for (const cb of listeners) cb(value);
    },
  };
  return stub;
}

/** Minimal harness that replaces `window.parent.postMessage` with a recorder. */
function makeHarness() {
  const outbound: EditorOutboundMessage[] = [];
  let listener: MessageListener | null = null;

  const post = (msg: EditorOutboundMessage) => {
    outbound.push(msg);
  };
  const subscribe = (cb: MessageListener) => {
    listener = cb;
    return {
      dispose: () => {
        listener = null;
      },
    };
  };
  const sendFromParent = (msg: EditorInboundMessage) => {
    if (listener !== null) listener(msg);
  };
  return { outbound, post, subscribe, sendFromParent };
}

describe('postMessage protocol', () => {
  it('emits `ready` on startup', () => {
    const editor = makeStubEditor();
    const h = makeHarness();
    const handles = createIframeProtocol({ editor, post: h.post, subscribe: h.subscribe });

    expect(h.outbound[0]).toEqual({ kind: 'ready' });
    handles.dispose();
  });

  it('round-trips `open`: parent sends -> editor.setValue called -> `opened` emitted', () => {
    const editor = makeStubEditor();
    const h = makeHarness();
    const handles = createIframeProtocol({ editor, post: h.post, subscribe: h.subscribe });
    h.outbound.length = 0;

    h.sendFromParent({
      kind: 'open',
      uri: 'file:///tmp/foo.ts',
      languageId: 'typescript',
      value: 'const x = 1;\n',
    });

    expect(editor.setValueCalls).toEqual(['const x = 1;\n']);
    expect(editor.getValue()).toBe('const x = 1;\n');
    expect(h.outbound).toEqual([{ kind: 'opened', uri: 'file:///tmp/foo.ts' }]);

    handles.dispose();
  });

  it('editor edits emit `change` to the parent', () => {
    const editor = makeStubEditor();
    const h = makeHarness();
    const handles = createIframeProtocol({ editor, post: h.post, subscribe: h.subscribe });

    h.sendFromParent({ kind: 'open', uri: 'file:///a.txt', languageId: 'plaintext', value: 'a' });
    h.outbound.length = 0;

    editor.emitChange('ab');

    expect(h.outbound).toEqual([{ kind: 'change', uri: 'file:///a.txt', value: 'ab' }]);
    handles.dispose();
  });

  it('`save` echoes current buffer back to the parent', () => {
    const editor = makeStubEditor();
    const h = makeHarness();
    const handles = createIframeProtocol({ editor, post: h.post, subscribe: h.subscribe });

    h.sendFromParent({ kind: 'open', uri: 'file:///b.md', languageId: 'markdown', value: 'hello' });
    editor.emitChange('hello world');
    h.outbound.length = 0;

    h.sendFromParent({ kind: 'save', uri: 'file:///b.md' });

    expect(h.outbound).toEqual([{ kind: 'save', uri: 'file:///b.md', value: 'hello world' }]);
    handles.dispose();
  });

  it('`close` clears the active URI and notifies the parent', () => {
    const editor = makeStubEditor();
    const h = makeHarness();
    const handles = createIframeProtocol({ editor, post: h.post, subscribe: h.subscribe });

    h.sendFromParent({ kind: 'open', uri: 'file:///c.txt', languageId: 'plaintext', value: 'hi' });
    h.outbound.length = 0;

    h.sendFromParent({ kind: 'close', uri: 'file:///c.txt' });

    expect(handles.currentUri()).toBeNull();
    expect(editor.getValue()).toBe('');
    expect(h.outbound).toEqual([{ kind: 'closed', uri: 'file:///c.txt' }]);
    handles.dispose();
  });

  it('`focus` calls editor.focus but emits nothing', () => {
    const editor = makeStubEditor();
    const h = makeHarness();
    const handles = createIframeProtocol({ editor, post: h.post, subscribe: h.subscribe });
    h.outbound.length = 0;

    h.sendFromParent({ kind: 'focus' });

    expect(editor.focusCalls).toBe(1);
    expect(h.outbound).toEqual([]);
    handles.dispose();
  });

  it('iframe-side LSP request (has id) posts `client.message`', () => {
    const editor = makeStubEditor();
    const h = makeHarness();
    const handles = createIframeProtocol({ editor, post: h.post, subscribe: h.subscribe });
    h.outbound.length = 0;

    handles.socket.send(JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'initialize' }));

    expect(h.outbound).toEqual([
      { kind: 'client.message', payload: { jsonrpc: '2.0', id: 1, method: 'initialize' } },
    ]);
    handles.dispose();
  });

  it('iframe-side LSP notification (no id) posts `client.notification`', () => {
    const editor = makeStubEditor();
    const h = makeHarness();
    const handles = createIframeProtocol({ editor, post: h.post, subscribe: h.subscribe });
    h.outbound.length = 0;

    handles.socket.send(
      JSON.stringify({
        jsonrpc: '2.0',
        method: 'textDocument/didOpen',
        params: { textDocument: { uri: 'file:///x.rs', languageId: 'rust', version: 0, text: '' } },
      }),
    );

    expect(h.outbound).toEqual([
      {
        kind: 'client.notification',
        payload: {
          jsonrpc: '2.0',
          method: 'textDocument/didOpen',
          params: {
            textDocument: { uri: 'file:///x.rs', languageId: 'rust', version: 0, text: '' },
          },
        },
      },
    ]);
    handles.dispose();
  });

  it('parent -> `client.message` delivers to socket subscribers', () => {
    const editor = makeStubEditor();
    const h = makeHarness();
    const handles = createIframeProtocol({ editor, post: h.post, subscribe: h.subscribe });
    const sink = vi.fn();
    handles.socket.onMessage(sink);

    h.sendFromParent({
      kind: 'client.message',
      payload: { jsonrpc: '2.0', id: 1, result: null },
    });

    expect(sink).toHaveBeenCalledTimes(1);
    expect(sink.mock.calls[0]?.[0]).toBe('{"jsonrpc":"2.0","id":1,"result":null}');
    handles.dispose();
  });

  it('parent -> `client.notification` delivers to the same subscribers (LSP bidirectional)', () => {
    const editor = makeStubEditor();
    const h = makeHarness();
    const handles = createIframeProtocol({ editor, post: h.post, subscribe: h.subscribe });
    const sink = vi.fn();
    handles.socket.onMessage(sink);

    h.sendFromParent({
      kind: 'client.notification',
      payload: {
        jsonrpc: '2.0',
        method: 'window/logMessage',
        params: { type: 3, message: 'hello' },
      },
    });

    expect(sink).toHaveBeenCalledTimes(1);
    handles.dispose();
  });

  it('ignores malformed messages silently', () => {
    const editor = makeStubEditor();
    const h = makeHarness();
    const handles = createIframeProtocol({ editor, post: h.post, subscribe: h.subscribe });
    h.outbound.length = 0;

    // `listener` is typed as `unknown`, so we send a nonsense shape through.
    (h as unknown as { sendFromParent: (v: unknown) => void }).sendFromParent({ nope: true });
    (h as unknown as { sendFromParent: (v: unknown) => void }).sendFromParent(null);
    (h as unknown as { sendFromParent: (v: unknown) => void }).sendFromParent('not an object');

    expect(editor.setValueCalls).toEqual([]);
    expect(h.outbound).toEqual([]);
    handles.dispose();
  });
});
