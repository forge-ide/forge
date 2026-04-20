// F-121: iframe entry point.
//
// Mounts Monaco into `#editor`, wires the postMessage protocol, and
// constructs (but does not start) the `MonacoLanguageClient`.

import { mountEditor } from './editor.js';
import { createLanguageClient } from './client.js';
import {
  browserPost,
  browserSubscribe,
  createIframeProtocol,
} from './protocol.js';

const host = document.getElementById('editor');
if (host === null) {
  throw new Error('monaco-host: #editor element not found');
}

const editor = mountEditor(host);

const handles = createIframeProtocol({
  editor,
  post: browserPost(),
  subscribe: browserSubscribe(),
});

// Construct the LSP client eagerly so the transport path is verified at
// boot. Starting it is F-123's job; see README "LSP lifecycle".
const client = createLanguageClient(handles.socket);

// Expose for debugging only; keep off `window` in production builds.
if (import.meta.env.DEV) {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  (window as any).__forgeMonacoHost = { handles, client };
}
