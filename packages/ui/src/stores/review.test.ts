import { afterEach, expect, it, vi } from "vitest";
import type { Changeset, FileChange } from "@forge/client";
import { fakeDaemon, settle } from "../testing/daemon";
import { createReviewStore } from "./review";
const file: FileChange = {
  repo: "repo",
  path: '"a\\376.sh"',
  status: "modified",
  additions: 2,
  deletions: 1,
  binary: false,
  tooLarge: false,
  viewed: false,
};
const changeset: Changeset = {
  sessionId: "ses_test",
  head: "cp_reviewed",
  base: "base",
  round: 2,
  lastReviewed: "cp_old",
  files: [file],
};
afterEach(() => vi.useRealTimers());
function setup() {
  const daemon = fakeDaemon((method) =>
    method === "review.changeset"
      ? changeset
      : method === "review.threads"
        ? []
        : method === "review.fileDiff"
          ? { hunks: [] }
          : method === "session.get"
            ? { id: "ses_test", title: "Test", state: "review" }
            : {},
  );
  return { ...daemon, store: createReviewStore(daemon.conn, "ses_test") };
}
it("preserves quoted paths and the displayed checkpoint for diff and revert", async () => {
  const { store, calls } = setup();
  await settle();
  await store.select(file);
  await store.setSinceLast(true);
  await store.revert("hunk");
  expect(calls.filter((c) => c.method === "review.fileDiff").at(-1)?.params).toEqual({
    sessionId: "ses_test",
    repo: "repo",
    path: file.path,
    fromCheckpoint: "cp_old",
  });
  expect(calls.find((c) => c.method === "review.revertHunk")?.params).toEqual({
    sessionId: "ses_test",
    repo: "repo",
    path: file.path,
    fromCheckpoint: "cp_old",
    hunkId: "hunk",
  });
  store.dispose();
});
it("debounces thread bursts without reloading the diff", async () => {
  vi.useFakeTimers();
  const { store, calls, emit } = setup();
  await settle();
  const before = calls.length;
  for (let i = 0; i < 100; i++) emit("review.thread");
  await vi.advanceTimersByTimeAsync(60);
  expect(calls.slice(before).map((c) => c.method)).toEqual(["review.threads"]);
  store.dispose();
});
it("reloads viewed marks authoritatively and ships the captured reviewed head", async () => {
  const { store, calls } = setup();
  await settle();
  await store.setViewed(file, true);
  expect(calls.at(-1)?.method).toBe("review.changeset");
  await store.ship("cp_reviewed", "push-branch", "branch", false);
  expect(calls.at(-1)?.params).toEqual({
    sessionId: "ses_test",
    checkpointId: "cp_reviewed",
    strategy: "push-branch",
    branch: "branch",
  });
  store.dispose();
});
it("retains stale data and releases listeners on disposal", async () => {
  const { store, setState, listenerCount } = setup();
  await settle();
  setState({ kind: "reconnecting" });
  expect(store.state.stale).toBe(true);
  expect(store.state.changeset?.head).toBe("cp_reviewed");
  store.dispose();
  expect(listenerCount()).toBe(0);
});
it("does not let a late diff replace a different file", async () => {
  let finish: (value: unknown) => void = () => {};
  const d = fakeDaemon((method, params) =>
    method === "review.changeset"
      ? changeset
      : method === "review.threads"
        ? []
        : method === "review.fileDiff"
          ? (params as { path: string }).path === file.path
            ? new Promise((r) => {
                finish = r;
              })
            : { hunks: [{ id: "new" }] }
          : {},
  );
  const store = createReviewStore(d.conn, "ses_test");
  await settle();
  const pending = store.select(file);
  await settle();
  await store.select({ ...file, path: "other" });
  finish({ hunks: [{ id: "old" }] });
  await pending;
  expect(store.state.hunks[0]?.id).toBe("new");
  store.dispose();
});
it("marks refused diffs withheld and never treats them as an empty review", async () => {
  const { DaemonError } = await import("@forge/client");
  const d = fakeDaemon((method) =>
    method === "review.changeset"
      ? changeset
      : method === "review.threads"
        ? []
        : method === "review.fileDiff"
          ? Promise.reject(
              new DaemonError(-32008, "too many lines", {
                kind: "ValidationFailed",
                detail: "too many lines",
                retryable: false,
              }),
            )
          : {},
  );
  const s = createReviewStore(d.conn, "ses_test");
  await settle();
  expect(s.state.diffStatus).toBe("withheld");
  expect(s.state.withheld).toContain(JSON.stringify(["repo", file.path]));
  s.dispose();
});
it("keeps full changeset withheld files visible when comparing since last review", async () => {
  const d = fakeDaemon((method, params) =>
    method === "review.changeset"
      ? (params as { fromCheckpoint?: string }).fromCheckpoint
        ? { ...changeset, files: [] }
        : { ...changeset, files: [{ ...file, tooLarge: true }] }
      : method === "review.threads"
        ? []
        : {},
  );
  const s = createReviewStore(d.conn, "ses_test");
  await settle();
  await s.setSinceLast(true);
  expect(s.state.changeset?.files).toEqual([]);
  expect(s.state.fullChangeset?.files[0]?.tooLarge).toBe(true);
  s.dispose();
});
it("retries a diff interrupted by a disconnect even when the checkpoint is unchanged", async () => {
  const d = fakeDaemon((method) =>
    method === "review.changeset"
      ? changeset
      : method === "review.threads"
        ? []
        : method === "review.fileDiff"
          ? new Promise(() => {})
          : {},
  );
  const s = createReviewStore(d.conn, "ses_test");
  await settle();
  d.setState({ kind: "reconnecting" });
  d.setState({
    kind: "connected",
    hello: { daemonVersion: "0", apiVersion: "0.5.0", platform: "linux", sandboxTiers: [] },
  });
  await settle();
  expect(d.calls.filter((c) => c.method === "review.fileDiff")).toHaveLength(2);
  s.dispose();
});
it("moves the since-last baseline when a new review round is recorded", async () => {
  let last = "cp_old";
  const d = fakeDaemon((method) =>
    method === "review.changeset"
      ? { ...changeset, lastReviewed: last }
      : method === "review.threads"
        ? []
        : method === "review.fileDiff"
          ? { hunks: [] }
          : {},
  );
  const s = createReviewStore(d.conn, "ses_test");
  await settle();
  await s.setSinceLast(true);
  last = "cp_reviewed";
  await s.refresh();
  expect(s.state.fromCheckpoint).toBe("cp_reviewed");
  s.dispose();
});
it("refuses to publish mismatched full and scoped checkpoint inventories", async () => {
  const d = fakeDaemon((method, params) =>
    method === "review.changeset"
      ? (params as { fromCheckpoint?: string }).fromCheckpoint
        ? { ...changeset, head: "cp_new" }
        : changeset
      : method === "review.threads"
        ? []
        : method === "review.fileDiff"
          ? { hunks: [] }
          : {},
  );
  const s = createReviewStore(d.conn, "ses_test");
  await settle();
  await s.setSinceLast(true);
  expect(s.state.status).toBe("error");
  expect(s.state.changeset?.head).toBe("cp_reviewed");
  expect(s.state.fullChangeset?.head).toBe("cp_reviewed");
  s.dispose();
});
it("does not replace stale review content when a queued refresh fires after disconnect", async () => {
  vi.useFakeTimers();
  let connected = true;
  const d = fakeDaemon((method) =>
    !connected
      ? Promise.reject(new Error("not connected"))
      : method === "review.changeset"
        ? changeset
        : method === "review.threads"
          ? []
          : method === "review.fileDiff"
            ? { hunks: [] }
            : {},
  );
  const s = createReviewStore(d.conn, "ses_test");
  await settle();
  d.emit("checkpoint");
  connected = false;
  d.setState({ kind: "reconnecting" });
  await vi.advanceTimersByTimeAsync(60);
  expect(s.state.status).toBe("ready");
  expect(s.state.stale).toBe(true);
  expect(s.state.changeset?.head).toBe("cp_reviewed");
  s.dispose();
});
it("blocks direct shipping after mixed snapshots and recovers after a consistent retry", async () => {
  let mismatch = false;
  const d = fakeDaemon((method, params) =>
    method === "review.changeset"
      ? (params as { fromCheckpoint?: string }).fromCheckpoint
        ? { ...changeset, head: mismatch ? "cp_new" : changeset.head }
        : changeset
      : method === "review.threads"
        ? []
        : method === "review.fileDiff"
          ? { hunks: [] }
          : {},
  );
  const s = createReviewStore(d.conn, "ses_test");
  await settle();
  mismatch = true;
  await s.setSinceLast(true);
  await expect(s.ship("cp_reviewed", "push-branch", "", true)).rejects.toThrow(/reload/i);
  expect(d.calls.some((c) => c.method === "review.ship")).toBe(false);
  mismatch = false;
  await s.refresh();
  await s.ship("cp_reviewed", "push-branch", "", false);
  expect(d.calls.at(-1)?.params).toEqual({
    sessionId: "ses_test",
    checkpointId: "cp_reviewed",
    strategy: "push-branch",
  });
  s.dispose();
});
it("also rejects an older scoped snapshot paired with a newer full snapshot", async () => {
  let mismatch = false;
  const d = fakeDaemon((method, params) =>
    method === "review.changeset"
      ? {
          ...changeset,
          head:
            mismatch && !(params as { fromCheckpoint?: string }).fromCheckpoint
              ? "cp_new"
              : changeset.head,
        }
      : method === "review.threads"
        ? []
        : method === "review.fileDiff"
          ? { hunks: [] }
          : {},
  );
  const s = createReviewStore(d.conn, "ses_test");
  await settle();
  await s.setSinceLast(true);
  mismatch = true;
  await s.refresh();
  expect(s.state.status).toBe("error");
  expect(s.canShip("cp_reviewed")).toBe(false);
  expect(s.state.fullChangeset?.head).toBe("cp_reviewed");
  s.dispose();
});
