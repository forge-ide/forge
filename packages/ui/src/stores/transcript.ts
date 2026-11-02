import { createStore } from "solid-js/store";
import type { Event as DaemonEvent } from "@forge/client";
import type { DaemonConnection } from "../client/connection";
import { errorText } from "./review";
export type TranscriptRow = { id: number; label: string; text: string; detail: boolean };
function transcriptRows(events: DaemonEvent[], turnId: string): TranscriptRow[] {
  const rows: TranscriptRow[] = [];
  const messages = new Map<string, TranscriptRow>();
  const tools = new Map<string, TranscriptRow>();
  const selectedEvents = new Set(events.map((event) => event.seq));
  for (const event of events) {
    if (event.parent !== undefined && !selectedEvents.has(event.parent)) continue;
    if ("turnId" in event.body && event.body.turnId !== turnId) continue;
    switch (event.kind) {
      case "prompt":
        rows.push({ id: event.seq, label: "Human prompt", text: event.body.text, detail: false });
        break;
      case "agent.message": {
        const existing = messages.get(event.body.messageId);
        if (existing) existing.text += event.body.delta;
        else {
          const row = { id: event.seq, label: "Agent", text: event.body.delta, detail: false };
          rows.push(row);
          messages.set(event.body.messageId, row);
        }
        break;
      }
      case "tool.call": {
        const row = {
          id: event.seq,
          label: `Tool · ${event.body.source}`,
          text: `${event.body.title}\n${JSON.stringify(event.body.action, null, 2)}`,
          detail: true,
        };
        rows.push(row);
        tools.set(event.body.toolCallId, row);
        break;
      }
      case "tool.result":
      case "policy.decision": {
        const label =
          event.kind === "tool.result"
            ? `Result · ${event.body.status}`
            : `Policy · ${event.body.verdict}`;
        const text =
          event.kind === "tool.result" ? (event.body.output ?? "(no output)") : event.body.reason;
        const tool = tools.get(event.body.toolCallId);
        if (tool) tool.text += `\n${label}\n${text}`;
        else rows.push({ id: event.seq, label, text, detail: true });
        break;
      }
      case "terminal.output":
        rows.push({
          id: event.seq,
          label: `Terminal output · ${event.body.terminalId}`,
          text: event.body.chunk,
          detail: true,
        });
        break;
    }
  }
  return rows;
}
export function createTranscriptStore(conn: DaemonConnection, sessionId: string, turnId: string) {
  const [state, set] = createStore<{
    status: "loading" | "ready" | "error";
    rows: TranscriptRow[];
    partial: boolean;
    stale: boolean;
    error: string | undefined;
    pages: number;
  }>({
    status: "loading",
    rows: [],
    partial: true,
    stale: conn.state.kind !== "connected",
    error: undefined,
    pages: 0,
  });
  let generation = 0,
    disposed = false;
  async function refresh() {
    const gen = ++generation;
    set({ status: "loading", error: undefined, pages: 0 });
    let events: DaemonEvent[] = [],
      before: number | undefined;
    try {
      while (!disposed && gen === generation) {
        const page = await conn.call("events.query", {
          sessionId,
          limit: 1000,
          ...(before === undefined ? {} : { before }),
        });
        if (disposed || gen !== generation) return;
        set("pages", (n) => n + 1);
        events = [...page, ...events];
        const start = events.findIndex(
          (e) => e.kind === "turn.started" && e.body.turnId === turnId,
        );
        if (start >= 0) {
          const after = events.slice(start + 1);
          const boundary = after.findIndex(
            (e) =>
              (e.kind === "turn.ended" && e.body.turnId === turnId) || e.kind === "turn.started",
          );
          const end = boundary < 0 ? after.length : boundary;
          const complete = boundary >= 0 && after[boundary]?.kind === "turn.ended";
          set({
            status: "ready",
            rows: transcriptRows(after.slice(0, end), turnId),
            partial: !complete,
          });
          return;
        }
        const next = page[0]?.seq;
        if (next === undefined || next === before) {
          set({ status: "ready", rows: [], partial: true });
          return;
        }
        before = next;
      }
    } catch (e) {
      if (!disposed && gen === generation) set({ status: "error", error: errorText(e) });
    }
  }
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
      off();
    },
  };
}
