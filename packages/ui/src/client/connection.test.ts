import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { API_VERSION, type EventNotification } from "@forge/client";
import type { DaemonTransport } from "../host/daemon";
import { DaemonConnection, type ConnectionState } from "./connection";

type Request = { id: number; method: string; params: Record<string, unknown> };

/** A scripted daemon on the other side of the transport. */
class FakeTransport implements DaemonTransport {
  up = true;
  apiVersion = API_VERSION;
  connects = 0;
  /** Accept requests but never answer them, like a daemon that dies mid-call. */
  holdReplies = false;
  requests: Request[] = [];
  private message = new Set<(text: string) => void>();
  private close = new Set<(reason: string) => void>();
  private nextSub = 1;

  connect(): Promise<void> {
    this.connects++;
    return this.up ? Promise.resolve() : Promise.reject(new Error("socket missing"));
  }

  send(text: string): Promise<void> {
    if (!this.up) return Promise.reject(new Error("not connected"));
    const req = JSON.parse(text) as Request;
    this.requests.push(req);
    if (!this.holdReplies) queueMicrotask(() => this.reply(req));
    return Promise.resolve();
  }

  onMessage(listener: (text: string) => void): () => void {
    this.message.add(listener);
    return () => this.message.delete(listener);
  }

  onClose(listener: (reason: string) => void): () => void {
    this.close.add(listener);
    return () => this.close.delete(listener);
  }

  info() {
    return Promise.resolve({ socket: "/run/forged.sock", log: "/data/forged.log" });
  }

  drop() {
    this.up = false;
    for (const l of this.close) l("daemon closed the connection");
  }

  push(note: EventNotification) {
    this.emit({ jsonrpc: "2.0", method: "event", params: note });
  }

  subscribeRequests() {
    return this.requests.filter((r) => r.method === "events.subscribe");
  }

  private reply(req: Request) {
    let result: unknown = {};
    if (req.method === "server.hello") {
      result = {
        daemonVersion: "0.0.0",
        apiVersion: this.apiVersion,
        platform: "linux",
        sandboxTiers: [],
      };
    } else if (req.method === "events.subscribe") {
      result = { subscriptionId: `sub_${this.nextSub++}` };
    } else if (req.method === "project.list") {
      result = [];
    }
    this.emit({ jsonrpc: "2.0", id: req.id, result });
  }

  private emit(message: unknown) {
    const text = JSON.stringify(message);
    for (const l of this.message) l(text);
  }
}

function note(subscriptionId: string, seq: number): EventNotification {
  return {
    subscriptionId,
    topic: "board",
    seq,
    event: {
      seq,
      sessionId: "ses_01J00000000000000000000000",
      at: "2026-10-23T12:00:00Z",
      kind: "usage",
      body: { inputTokens: seq, outputTokens: 0 },
    },
  };
}

function track(conn: DaemonConnection): ConnectionState[] {
  const states: ConnectionState[] = [];
  conn.onStateChange((s) => states.push(s));
  return states;
}

describe("DaemonConnection", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("connects and says hello", async () => {
    const transport = new FakeTransport();
    const conn = new DaemonConnection(transport);
    conn.start();
    await vi.runAllTimersAsync();
    expect(conn.state.kind).toBe("connected");
    expect(transport.requests[0]?.method).toBe("server.hello");
    expect(transport.requests[0]?.params.apiVersion).toBe(API_VERSION);
  });

  it("keeps retrying while the daemon starts, then fails after 10 s with the log path", async () => {
    const transport = new FakeTransport();
    transport.up = false;
    const conn = new DaemonConnection(transport);
    conn.start();
    await vi.advanceTimersByTimeAsync(5_000);
    expect(conn.state.kind).toBe("starting");
    expect(transport.connects).toBeGreaterThan(2);

    await vi.advanceTimersByTimeAsync(6_000);
    const state = conn.state;
    expect(state.kind).toBe("failed");
    if (state.kind === "failed") {
      expect(state.error).toContain("socket missing");
      expect(state.logPath).toBe("/data/forged.log");
    }

    const attempts = transport.connects;
    await vi.advanceTimersByTimeAsync(30_000);
    expect(transport.connects).toBe(attempts);

    transport.up = true;
    conn.retry();
    await vi.runAllTimersAsync();
    expect(conn.state.kind).toBe("connected");
  });

  it("refuses a daemon with a different major API version", async () => {
    const transport = new FakeTransport();
    transport.apiVersion = "9.0.0";
    const conn = new DaemonConnection(transport);
    conn.start();
    await vi.runAllTimersAsync();
    const state = conn.state;
    expect(state.kind).toBe("failed");
    if (state.kind === "failed") expect(state.error).toContain("9.0.0");
  });

  it("reconnects after a drop and resubscribes with afterSeq", async () => {
    const transport = new FakeTransport();
    const conn = new DaemonConnection(transport);
    const states = track(conn);
    const seen: number[] = [];
    conn.subscribe(["board"], (n) => seen.push(n.seq));
    conn.start();
    await vi.runAllTimersAsync();

    const [first] = transport.subscribeRequests();
    expect(first?.params).toEqual({ topics: ["board"] });
    transport.push(note("sub_1", 1));
    transport.push(note("sub_1", 2));
    expect(seen).toEqual([1, 2]);

    transport.drop();
    expect(conn.state.kind).toBe("reconnecting");
    await vi.advanceTimersByTimeAsync(1_000);
    expect(conn.state.kind).toBe("reconnecting");

    transport.up = true;
    await vi.runAllTimersAsync();
    expect(conn.state.kind).toBe("connected");
    const second = transport.subscribeRequests()[1];
    expect(second?.params).toEqual({ topics: ["board"], afterSeq: 2 });

    // Events for the old subscription ID are ignored; the new one delivers.
    transport.push(note("sub_1", 3));
    transport.push(note("sub_2", 3));
    expect(seen).toEqual([1, 2, 3]);
    expect(states.map((s) => s.kind)).toEqual(["connected", "reconnecting", "connected"]);
  });

  it("retries when the connection drops before hello is answered", async () => {
    const transport = new FakeTransport();
    transport.holdReplies = true;
    const conn = new DaemonConnection(transport);
    conn.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(transport.requests.map((r) => r.method)).toEqual(["server.hello"]);

    transport.drop();
    transport.up = true;
    transport.holdReplies = false;
    await vi.runAllTimersAsync();
    expect(transport.connects).toBe(2);
    expect(conn.state.kind).toBe("connected");
  });

  it("rejects calls while disconnected", async () => {
    const transport = new FakeTransport();
    transport.up = false;
    const conn = new DaemonConnection(transport);
    await expect(conn.call("project.list", {})).rejects.toThrow("not connected");
  });

  it("stops delivering after unsubscribe", async () => {
    const transport = new FakeTransport();
    const conn = new DaemonConnection(transport);
    const seen: number[] = [];
    const sub = conn.subscribe(["board"], (n) => seen.push(n.seq));
    conn.start();
    await vi.runAllTimersAsync();
    sub.unsubscribe();
    await vi.runAllTimersAsync();
    expect(transport.requests.some((r) => r.method === "events.unsubscribe")).toBe(true);
    transport.push(note("sub_1", 1));
    expect(seen).toEqual([]);
  });
});
