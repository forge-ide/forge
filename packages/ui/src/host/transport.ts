export type DaemonInfo = { socket: string; log: string };

/** Moves JSON-RPC text frames between the UI and the daemon. */
export interface DaemonTransport {
  /** Open a connection; the desktop host starts the daemon if needed. */
  connect(): Promise<void>;
  send(text: string): Promise<void>;
  onMessage(listener: (text: string) => void): () => void;
  /** Called once per connection when it ends. */
  onClose(listener: (reason: string) => void): () => void;
  info(): Promise<DaemonInfo>;
}
