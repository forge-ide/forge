import { expect, it } from "vitest";
import { fakeDaemon, settle } from "../testing/daemon";
import { createTranscriptStore } from "./transcript";
it("loads older pages to the chosen turn and coalesces its streamed messages", async () => {
  const d = fakeDaemon((_method, params) =>
    "before" in (params as object)
      ? [
          { seq: 1, kind: "turn.started", body: { turnId: "turn_wanted" } },
          {
            seq: 2,
            kind: "agent.message",
            body: { turnId: "turn_wanted", messageId: "msg", delta: "hello " },
          },
        ]
      : [
          {
            seq: 3,
            kind: "agent.message",
            body: { turnId: "turn_wanted", messageId: "msg", delta: "world" },
          },
          { seq: 4, kind: "turn.ended", body: { turnId: "turn_wanted" } },
          { seq: 5, kind: "turn.started", body: { turnId: "other" } },
        ],
  );
  const store = createTranscriptStore(d.conn, "ses_test", "turn_wanted");
  await settle();
  expect(store.state.rows.map((r) => r.text)).toContain("hello world");
  expect(store.state.partial).toBe(false);
  expect(d.calls[1]?.params).toEqual({ sessionId: "ses_test", limit: 1000, before: 3 });
  store.dispose();
});
it("labels missing start boundaries as incomplete and ignores late results after disposal", async () => {
  const d = fakeDaemon(() => []);
  const store = createTranscriptStore(d.conn, "ses_test", "missing");
  await settle();
  expect(store.state.status).toBe("ready");
  expect(store.state.partial).toBe(true);
  expect(store.state.rows).toEqual([]);
  store.dispose();
  expect(d.listenerCount()).toBe(0);
});
it("associates policy and result output with its tool and excludes other turns", async () => {
  const d = fakeDaemon(() => [
    { seq: 1, kind: "turn.started", body: { turnId: "chosen" } },
    {
      seq: 2,
      kind: "tool.call",
      body: {
        toolCallId: "tool",
        source: "agent",
        title: "Read file",
        action: { type: "exec", argv: ["cat", "a"], cwd: "." },
      },
    },
    {
      seq: 3,
      kind: "policy.decision",
      body: { toolCallId: "tool", verdict: "allow", reason: "read only" },
    },
    {
      seq: 4,
      kind: "agent.message",
      body: { turnId: "other", messageId: "other", delta: "UNRELATED" },
    },
    {
      seq: 5,
      kind: "tool.result",
      body: { toolCallId: "tool", status: "completed", output: "OUTPUT" },
    },
    { seq: 6, kind: "turn.ended", body: { turnId: "chosen" } },
  ]);
  const s = createTranscriptStore(d.conn, "ses_test", "chosen");
  await settle();
  expect(s.state.rows).toHaveLength(1);
  expect(s.state.rows[0]?.text).toContain("OUTPUT");
  expect(s.state.rows[0]?.text).toContain("read only");
  expect(s.state.rows[0]?.text).not.toContain("UNRELATED");
  s.dispose();
});
it("excludes results whose explicit parent is outside the selected turn and identifies terminals", async () => {
  const d = fakeDaemon(() => [
    { seq: 4, kind: "turn.started", body: { turnId: "chosen" } },
    {
      seq: 5,
      parent: 2,
      kind: "tool.result",
      body: { toolCallId: "old", status: "completed", output: "OLD TURN RESULT" },
    },
    { seq: 6, kind: "terminal.output", body: { terminalId: "terminal_a", chunk: "line" } },
    { seq: 7, kind: "turn.ended", body: { turnId: "chosen" } },
  ]);
  const s = createTranscriptStore(d.conn, "ses_test", "chosen");
  await settle();
  expect(s.state.rows.map((r) => r.text).join("")).not.toContain("OLD TURN RESULT");
  expect(s.state.rows[0]?.label).toContain("terminal_a");
  s.dispose();
});
