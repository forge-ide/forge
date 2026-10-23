// The daemon socket, reached through the Tauri shell. The only daemon code that touches Tauri.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type DaemonInfo = { socket: string; log: string };

/** Moves JSON-RPC text frames between the UI and the daemon. */
export interface DaemonTransport {
  /** Open a connection, starting the daemon if it isn't running. */
  connect(): Promise<void>;
  send(text: string): Promise<void>;
  onMessage(listener: (text: string) => void): () => void;
  /** Called once per connection when it ends. */
  onClose(listener: (reason: string) => void): () => void;
  info(): Promise<DaemonInfo>;
}

const MESSAGE_EVENT = "forge://daemon-message";
const CLOSED_EVENT = "forge://daemon-closed";

export function createTauriTransport(): DaemonTransport {
  const messageListeners = new Set<(text: string) => void>();
  const closeListeners = new Set<(reason: string) => void>();
  // Listen before the first connect so no frame arrives unheard.
  let listening: Promise<unknown> | undefined;
  const ensureListening = () =>
    (listening ??= Promise.all([
      listen<string>(MESSAGE_EVENT, (e) => {
        for (const l of messageListeners) l(e.payload);
      }),
      listen<string>(CLOSED_EVENT, (e) => {
        for (const l of closeListeners) l(e.payload);
      }),
    ]));

  return {
    async connect() {
      await ensureListening();
      await invoke("daemon_connect");
    },
    async send(text) {
      await invoke("daemon_send", { text });
    },
    onMessage(listener) {
      messageListeners.add(listener);
      return () => messageListeners.delete(listener);
    },
    onClose(listener) {
      closeListeners.add(listener);
      return () => closeListeners.delete(listener);
    },
    info() {
      return invoke<DaemonInfo>("daemon_info");
    },
  };
}
