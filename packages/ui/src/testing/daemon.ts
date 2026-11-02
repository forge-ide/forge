import type { EventNotification, MethodName, Params, Result } from "@forge/client";
import type { ConnectionState, DaemonConnection } from "../client/connection";
export const connected: ConnectionState = {
  kind: "connected",
  hello: { daemonVersion: "0", apiVersion: "0.5.0", platform: "linux", sandboxTiers: [] },
};
export function fakeDaemon(respond: (method: MethodName, params: unknown) => unknown) {
  let state = connected;
  const listeners = new Set<(s: ConnectionState) => void>();
  const events = new Set<(e: EventNotification) => void>();
  const calls: { method: MethodName; params: unknown }[] = [];
  const conn = {
    get state() {
      return state;
    },
    call<M extends MethodName>(method: M, params: Params<M>): Promise<Result<M>> {
      calls.push({ method, params });
      return Promise.resolve().then(() => respond(method, params)) as Promise<Result<M>>;
    },
    onStateChange(fn: (s: ConnectionState) => void) {
      listeners.add(fn);
      return () => listeners.delete(fn);
    },
    subscribe(_topics: unknown, fn: (e: EventNotification) => void) {
      events.add(fn);
      return { unsubscribe: () => events.delete(fn) };
    },
  } as unknown as DaemonConnection;
  return {
    conn,
    calls,
    setState(s: ConnectionState) {
      state = s;
      listeners.forEach((fn) => fn(s));
    },
    emit(kind: string) {
      events.forEach((fn) => fn({ event: { kind, sessionId: "ses_test" } } as EventNotification));
    },
    listenerCount: () => listeners.size + events.size,
  };
}
export const settle = async () => {
  for (let n = 0; n < 15; n++) await Promise.resolve();
};
