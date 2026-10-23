import { describe, expect, it } from "vitest";
import type { Project } from "@forge/client";
import type { ConnectionState, DaemonConnection } from "../client/connection";
import { createProjectsStore } from "./projects";

const project: Project = {
  id: "prj_01J00000000000000000000000",
  name: "demo",
  root: "/src/demo",
  repos: [{ name: "demo", path: ".", baseBranch: "main" }],
};

/** Just enough of DaemonConnection for the store. */
function fakeConnection(list: () => Promise<Project[]>) {
  let state: ConnectionState = { kind: "starting" };
  const listeners = new Set<(s: ConnectionState) => void>();
  const conn = {
    get state() {
      return state;
    },
    onStateChange(l: (s: ConnectionState) => void) {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    call: () => list(),
  };
  const set = (next: ConnectionState) => {
    state = next;
    for (const l of listeners) l(next);
  };
  return { conn: conn as unknown as DaemonConnection, set };
}

const connected: ConnectionState = {
  kind: "connected",
  hello: { daemonVersion: "0.0.0", apiVersion: "0.2.0", platform: "linux", sandboxTiers: [] },
};

const settle = () => new Promise((r) => setTimeout(r, 0));

describe("projects store", () => {
  it("loads on connect, goes stale on disconnect, and reloads on reconnect", async () => {
    let projects: Project[] = [];
    const { conn, set } = fakeConnection(() => Promise.resolve(projects));
    const store = createProjectsStore(conn);
    expect(store.state.status).toBe("loading");

    set(connected);
    await settle();
    expect(store.state.status).toBe("ready");
    expect(store.state.projects).toEqual([]);

    set({ kind: "reconnecting" });
    expect(store.state.stale).toBe(true);

    projects = [project];
    set(connected);
    await settle();
    expect(store.state.stale).toBe(false);
    expect(store.state.projects).toEqual([project]);
  });

  it("shows an error while connected and clears it on a successful retry", async () => {
    let fail = true;
    const { conn, set } = fakeConnection(() =>
      fail ? Promise.reject(new Error("boom")) : Promise.resolve([project]),
    );
    const store = createProjectsStore(conn);
    set(connected);
    await settle();
    expect(store.state.status).toBe("error");
    expect(store.state.error).toBe("boom");

    fail = false;
    await store.refresh();
    expect(store.state.status).toBe("ready");
    expect(store.state.error).toBeUndefined();
  });
});
