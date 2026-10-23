// JSON-RPC 2.0 over any text transport. Correlates responses with calls and fans out
// event notifications; knows nothing about how frames travel.

import {
  EVENT_NOTIFICATION,
  type ErrorData,
  type ErrorKind,
  type EventNotification,
  type MethodName,
  type Methods,
} from "./generated/api";

export type Params<M extends MethodName> = Methods[M]["params"];
export type Result<M extends MethodName> = Methods[M]["result"];

/** An error response from the daemon. `kind` is set for application errors. */
export class DaemonError extends Error {
  readonly code: number;
  readonly data: ErrorData | undefined;

  constructor(code: number, message: string, data?: ErrorData) {
    super(message);
    this.name = "DaemonError";
    this.code = code;
    this.data = data;
  }

  get kind(): ErrorKind | undefined {
    return this.data?.kind;
  }
}

type Pending = { resolve: (value: unknown) => void; reject: (err: Error) => void };

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function toDaemonError(error: unknown): DaemonError {
  if (!isRecord(error)) return new DaemonError(0, "malformed error response");
  const code = typeof error.code === "number" ? error.code : 0;
  const message = typeof error.message === "string" ? error.message : "daemon error";
  const data = isRecord(error.data) ? (error.data as ErrorData) : undefined;
  return new DaemonError(code, message, data);
}

export class RpcClient {
  private nextId = 1;
  private readonly pending = new Map<number, Pending>();
  private readonly eventListeners = new Set<(n: EventNotification) => void>();

  constructor(private readonly send: (text: string) => Promise<void>) {}

  /** Call a daemon method. Results are trusted to match the generated types. */
  call<M extends MethodName>(method: M, params: Params<M>): Promise<Result<M>> {
    const id = this.nextId++;
    const text = JSON.stringify({ jsonrpc: "2.0", id, method, params });
    return new Promise<Result<M>>((resolve, reject) => {
      this.pending.set(id, { resolve: resolve as (value: unknown) => void, reject });
      this.send(text).catch((err: unknown) => {
        this.pending.delete(id);
        reject(err instanceof Error ? err : new Error(String(err)));
      });
    });
  }

  /** Subscribe to event notifications; returns an unsubscribe function. */
  onEvent(listener: (n: EventNotification) => void): () => void {
    this.eventListeners.add(listener);
    return () => this.eventListeners.delete(listener);
  }

  /** Handle one frame from the daemon. */
  receive(text: string): void {
    let message: unknown;
    try {
      message = JSON.parse(text);
    } catch {
      return;
    }
    if (!isRecord(message)) return;
    if (typeof message.id === "number") {
      const pending = this.pending.get(message.id);
      if (!pending) return;
      this.pending.delete(message.id);
      if ("error" in message) pending.reject(toDaemonError(message.error));
      else pending.resolve(message.result);
      return;
    }
    if (message.method === EVENT_NOTIFICATION && isRecord(message.params)) {
      const note = message.params as EventNotification;
      for (const listener of this.eventListeners) listener(note);
    }
  }

  /** Reject every in-flight call, e.g. when the connection drops. */
  failAll(err: Error): void {
    const pending = [...this.pending.values()];
    this.pending.clear();
    for (const p of pending) p.reject(err);
  }
}
