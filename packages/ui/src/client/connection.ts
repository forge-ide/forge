// The UI's single daemon connection: start-up retries, reconnects, and subscriptions that
// resume with `afterSeq` so a reconnect replays what was missed instead of reloading.

import {
  API_VERSION,
  RpcClient,
  type EventNotification,
  type HelloResult,
  type MethodName,
  type Params,
  type Result,
  type Topic,
} from "@forge/client";
import type { DaemonTransport } from "../host/daemon";

export type ConnectionState =
  /** Never connected yet; the daemon may still be starting. */
  | { kind: "starting"; lastError?: string }
  | { kind: "connected"; hello: HelloResult }
  /** Was connected and lost it; retrying in the background. */
  | { kind: "reconnecting"; lastError?: string }
  /** Gave up; `retry()` starts again. */
  | { kind: "failed"; error: string; logPath?: string | undefined };

export type Subscription = { unsubscribe(): void };

type Sub = {
  topics: Topic[];
  onEvent: (n: EventNotification) => void;
  lastSeq?: number;
  serverId?: string | undefined;
  closed: boolean;
};

export type ConnectionOptions = {
  /** How long start-up keeps retrying before showing an error. */
  giveUpAfterMs?: number;
  /** Delays between attempts; the last repeats. */
  backoffMs?: number[];
};

function majorOf(version: string): string {
  return version.split(".")[0] ?? "";
}

function message(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

export class DaemonConnection {
  private current: ConnectionState = { kind: "starting" };
  private rpc: RpcClient | undefined;
  private attempt = 0;
  private startedAt = 0;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private readonly subs = new Set<Sub>();
  private readonly listeners = new Set<(s: ConnectionState) => void>();
  private readonly giveUpAfterMs: number;
  private readonly backoffMs: number[];

  constructor(
    private readonly transport: DaemonTransport,
    options: ConnectionOptions = {},
  ) {
    this.giveUpAfterMs = options.giveUpAfterMs ?? 10_000;
    this.backoffMs = options.backoffMs ?? [250, 500, 1_000, 2_000, 5_000];
    transport.onMessage((text) => this.rpc?.receive(text));
    transport.onClose((reason) => this.lost(reason));
  }

  get state(): ConnectionState {
    return this.current;
  }

  onStateChange(listener: (s: ConnectionState) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  start(): void {
    this.startedAt = Date.now();
    this.attempt = 0;
    this.setState({ kind: "starting" });
    void this.tryConnect();
  }

  retry(): void {
    clearTimeout(this.timer);
    this.start();
  }

  call<M extends MethodName>(method: M, params: Params<M>): Promise<Result<M>> {
    if (this.current.kind !== "connected" || !this.rpc) {
      return Promise.reject(new Error("not connected to the daemon"));
    }
    return this.rpc.call(method, params);
  }

  /** Subscribe now if connected, and again after every reconnect. */
  subscribe(topics: Topic[], onEvent: (n: EventNotification) => void): Subscription {
    const sub: Sub = { topics, onEvent, closed: false };
    this.subs.add(sub);
    if (this.current.kind === "connected" && this.rpc) {
      void this.resubscribe(this.rpc, sub).catch(() => undefined);
    }
    return {
      unsubscribe: () => {
        sub.closed = true;
        this.subs.delete(sub);
        if (sub.serverId && this.rpc && this.current.kind === "connected") {
          void this.rpc
            .call("events.unsubscribe", { subscriptionId: sub.serverId })
            .catch(() => undefined);
        }
      },
    };
  }

  private async tryConnect(): Promise<void> {
    let rpc: RpcClient | undefined;
    try {
      await this.transport.connect();
      rpc = new RpcClient((text) => this.transport.send(text));
      rpc.onEvent((n) => this.route(n));
      this.rpc = rpc;
      const hello = await rpc.call("server.hello", {
        client: "forge-desktop",
        apiVersion: API_VERSION,
      });
      if (majorOf(hello.apiVersion) !== majorOf(API_VERSION)) {
        this.setState({
          kind: "failed",
          error: `The daemon speaks API ${hello.apiVersion}, but this app needs ${API_VERSION}. Restart forged from the same release.`,
        });
        return;
      }
      for (const sub of this.subs) await this.resubscribe(rpc, sub);
      this.attempt = 0;
      this.setState({ kind: "connected", hello });
    } catch (err) {
      rpc?.failAll(new Error("connection failed"));
      if (this.rpc === rpc) this.rpc = undefined;
      await this.scheduleRetry(message(err));
    }
  }

  private async resubscribe(rpc: RpcClient, sub: Sub): Promise<void> {
    const params =
      sub.lastSeq === undefined
        ? { topics: sub.topics }
        : { topics: sub.topics, afterSeq: sub.lastSeq };
    const { subscriptionId } = await rpc.call("events.subscribe", params);
    if (sub.closed) {
      void rpc.call("events.unsubscribe", { subscriptionId }).catch(() => undefined);
      return;
    }
    sub.serverId = subscriptionId;
  }

  private route(n: EventNotification): void {
    for (const sub of this.subs) {
      if (sub.serverId !== n.subscriptionId) continue;
      if (sub.lastSeq !== undefined && n.seq <= sub.lastSeq) return;
      sub.lastSeq = n.seq;
      sub.onEvent(n);
      return;
    }
  }

  private lost(reason: string): void {
    if (this.current.kind !== "connected") return;
    this.rpc?.failAll(new Error(reason));
    this.rpc = undefined;
    for (const sub of this.subs) sub.serverId = undefined;
    this.attempt = 0;
    this.setState({ kind: "reconnecting", lastError: reason });
    this.timer = setTimeout(() => void this.tryConnect(), this.delay());
  }

  private async scheduleRetry(error: string): Promise<void> {
    const starting = this.current.kind === "starting";
    if (starting && Date.now() - this.startedAt >= this.giveUpAfterMs) {
      const logPath = await this.transport.info().then(
        (info) => info.log,
        () => undefined,
      );
      this.setState({ kind: "failed", error, logPath });
      return;
    }
    this.setState(
      starting
        ? { kind: "starting", lastError: error }
        : { kind: "reconnecting", lastError: error },
    );
    // During start-up, make the last attempt land on the deadline rather than past it.
    const remaining = this.startedAt + this.giveUpAfterMs - Date.now();
    const delay = starting ? Math.min(this.delay(), remaining) : this.delay();
    this.timer = setTimeout(() => void this.tryConnect(), delay);
  }

  private delay(): number {
    const i = Math.min(this.attempt++, this.backoffMs.length - 1);
    return this.backoffMs[i] ?? 1_000;
  }

  private setState(next: ConnectionState): void {
    const changedKind = next.kind !== this.current.kind;
    this.current = next;
    if (changedKind || next.kind === "failed") {
      for (const l of this.listeners) l(next);
    }
  }
}
