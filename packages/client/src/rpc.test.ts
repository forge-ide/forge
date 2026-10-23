import { describe, expect, it } from "vitest";
import type { EventNotification, Project } from "./generated/api";
import { DaemonError, RpcClient } from "./rpc";

function setup() {
  const sent: unknown[] = [];
  const client = new RpcClient((text) => {
    sent.push(JSON.parse(text));
    return Promise.resolve();
  });
  return { client, sent };
}

const project: Project = {
  id: "prj_01J00000000000000000000000",
  name: "demo",
  root: "/src/demo",
  repos: [{ name: "demo", path: ".", baseBranch: "main" }],
};

describe("RpcClient", () => {
  it("sends JSON-RPC requests and resolves matching responses", async () => {
    const { client, sent } = setup();
    const result = client.call("project.list", {});
    expect(sent).toEqual([{ jsonrpc: "2.0", id: 1, method: "project.list", params: {} }]);
    client.receive(JSON.stringify({ jsonrpc: "2.0", id: 1, result: [project] }));
    await expect(result).resolves.toEqual([project]);
  });

  it("matches responses that arrive out of order", async () => {
    const { client } = setup();
    const first = client.call("project.list", {});
    const second = client.call("project.get", { projectId: project.id });
    client.receive(JSON.stringify({ jsonrpc: "2.0", id: 2, result: project }));
    client.receive(JSON.stringify({ jsonrpc: "2.0", id: 1, result: [] }));
    await expect(second).resolves.toEqual(project);
    await expect(first).resolves.toEqual([]);
  });

  it("rejects with a DaemonError carrying the application error", async () => {
    const { client } = setup();
    const result = client.call("project.get", { projectId: project.id });
    client.receive(
      JSON.stringify({
        jsonrpc: "2.0",
        id: 1,
        error: {
          code: -32001,
          message: "NotFound: project",
          data: { kind: "NotFound", detail: "project", retryable: false },
        },
      }),
    );
    const err: unknown = await result.catch((e: unknown) => e);
    expect(err).toBeInstanceOf(DaemonError);
    expect((err as DaemonError).kind).toBe("NotFound");
    expect((err as DaemonError).code).toBe(-32001);
  });

  it("delivers event notifications to listeners", () => {
    const { client } = setup();
    const seen: EventNotification[] = [];
    const off = client.onEvent((n) => seen.push(n));
    const note = {
      subscriptionId: "sub_01J00000000000000000000000",
      topic: "board",
      seq: 4,
      event: {
        seq: 4,
        sessionId: "ses_01J00000000000000000000000",
        at: "2026-10-23T12:00:00Z",
        kind: "usage",
        body: { inputTokens: 1, outputTokens: 2 },
      },
    };
    client.receive(JSON.stringify({ jsonrpc: "2.0", method: "event", params: note }));
    off();
    client.receive(JSON.stringify({ jsonrpc: "2.0", method: "event", params: note }));
    expect(seen).toEqual([note]);
  });

  it("rejects every pending call when the connection fails", async () => {
    const { client } = setup();
    const a = client.call("project.list", {});
    const b = client.call("server.hello", { client: "t", apiVersion: "0.1.0" });
    client.failAll(new Error("closed"));
    await expect(a).rejects.toThrow("closed");
    await expect(b).rejects.toThrow("closed");
  });

  it("rejects the call when sending fails", async () => {
    const client = new RpcClient(() => Promise.reject(new Error("not connected")));
    await expect(client.call("project.list", {})).rejects.toThrow("not connected");
  });

  it("ignores malformed frames", () => {
    const { client } = setup();
    expect(() => client.receive("not json")).not.toThrow();
    expect(() => client.receive(JSON.stringify({ id: 99, result: 1 }))).not.toThrow();
  });
});
