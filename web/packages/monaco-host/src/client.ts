// F-121: monaco-languageclient wiring.
//
// The client is constructed with a `MessageTransports` pair built around
// `IframeSocket` (our postMessage-backed `IWebSocket` adapter). The
// client is NOT started here — F-123 is responsible for wiring the parent
// to `forge-lsp` and instructing the iframe when to `.start()`. Until
// then `MonacoLanguageClient` is idle: it exposes the transport to prove
// the construction path works, nothing more. See README "LSP lifecycle".

import { MonacoLanguageClient } from 'monaco-languageclient';
import { CloseAction, ErrorAction } from 'vscode-languageclient/browser.js';
import { WebSocketMessageReader, WebSocketMessageWriter } from 'vscode-ws-jsonrpc';
import type { IframeSocket } from './protocol.js';

/**
 * Instantiate a `MonacoLanguageClient` bound to `socket`'s postMessage
 * transport. The returned client is not started.
 */
export function createLanguageClient(
  socket: IframeSocket,
  options: { name?: string; id?: string; documentSelector?: string[] } = {},
): MonacoLanguageClient {
  const reader = new WebSocketMessageReader(socket);
  const writer = new WebSocketMessageWriter(socket);

  return new MonacoLanguageClient({
    name: options.name ?? 'Forge Iframe LSP Client',
    id: options.id ?? 'forge-iframe-lsp',
    clientOptions: {
      documentSelector: options.documentSelector ?? ['plaintext'],
      errorHandler: {
        error: () => ({ action: ErrorAction.Continue }),
        closed: () => ({ action: CloseAction.DoNotRestart }),
      },
    },
    messageTransports: { reader, writer },
  });
}
