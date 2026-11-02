import { createStore } from "solid-js/store";
import type { Session } from "@forge/client";
import type { DaemonConnection } from "../client/connection";
import { errorText } from "./review";
export function createSessionsStore(conn: DaemonConnection, projectId: string) {
  const [state, set] = createStore<{
    status: "loading" | "ready" | "error";
    sessions: Session[];
    stale: boolean;
    error: string | undefined;
  }>({ status: "loading", sessions: [], stale: conn.state.kind !== "connected", error: undefined });
  let generation = 0,
    disposed = false,
    timer: ReturnType<typeof setTimeout> | undefined;
  async function refresh() {
    const gen = ++generation;
    try {
      const sessions = await conn.call("session.list", { projectId });
      if (!disposed && gen === generation)
        set({
          sessions,
          status: "ready",
          error: undefined,
          stale: conn.state.kind !== "connected",
        });
    } catch (e) {
      if (!disposed && gen === generation) set({ status: "error", error: errorText(e) });
    }
  }
  const sub = conn.subscribe(
    [`project/${projectId}`],
    () => {
      if (timer === undefined)
        timer = setTimeout(() => {
          timer = undefined;
          void refresh();
        }, 50);
    },
    () => void refresh(),
  );
  const off = conn.onStateChange((s) => {
    set("stale", s.kind !== "connected");
    if (s.kind === "connected") void refresh();
    else ++generation;
  });
  if (conn.state.kind === "connected") void refresh();
  return {
    state,
    refresh,
    dispose() {
      disposed = true;
      ++generation;
      clearTimeout(timer);
      off();
      sub.unsubscribe();
    },
  };
}
