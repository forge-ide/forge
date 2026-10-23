// Projects known to the daemon. Reloaded on every (re)connect; marked stale while disconnected.

import { createStore } from "solid-js/store";
import type { Project } from "@forge/client";
import type { DaemonConnection } from "../client/connection";

export type ProjectsState = {
  status: "loading" | "ready" | "error";
  projects: Project[];
  error?: string | undefined;
  /** True while disconnected: the list may be out of date. */
  stale: boolean;
};

export function createProjectsStore(conn: DaemonConnection) {
  const [state, setState] = createStore<ProjectsState>({
    status: "loading",
    projects: [],
    stale: false,
  });

  async function refresh(): Promise<void> {
    try {
      const projects = await conn.call("project.list", {});
      setState({ status: "ready", projects, stale: false, error: undefined });
    } catch (err) {
      // A disconnect mid-call is shown by the connection banner and retried on reconnect.
      if (conn.state.kind === "connected") {
        setState({ status: "error", error: err instanceof Error ? err.message : String(err) });
      }
    }
  }

  conn.onStateChange((s) => {
    if (s.kind === "connected") void refresh();
    else setState("stale", true);
  });
  if (conn.state.kind === "connected") void refresh();

  return { state, refresh };
}
