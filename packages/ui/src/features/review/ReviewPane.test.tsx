import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { Changeset } from "@forge/client";
import { fakeDaemon, settle } from "../../testing/daemon";
import { ReviewPane } from "./ReviewPane";
const cleanups: (() => void)[] = [];
afterEach(() => {
  cleanups.splice(0).forEach((fn) => fn());
  document.body.replaceChildren();
});
function mount(tooLarge = false) {
  let changeset: Changeset = {
    sessionId: "ses_test",
    head: "cp_reviewed",
    base: "base",
    round: 1,
    files: [
      {
        path: "<img src=x onerror=alert(1)>",
        status: "modified",
        additions: 2,
        deletions: 1,
        viewed: false,
        binary: false,
        tooLarge,
      },
    ],
  };
  const d = fakeDaemon((method) =>
    method === "review.changeset"
      ? changeset
      : method === "review.threads"
        ? [
            {
              id: "thr",
              sessionId: "ses_test",
              path: changeset.files[0]?.path ?? "",
              range: { startLine: 1, endLine: 1 },
              checkpointId: "cp_reviewed",
              quote: ["old\u202e"],
              status: "outdated",
              comments: [
                {
                  id: "c",
                  author: "agent",
                  body: "<img src=x onerror=alert(1)>\u202e",
                  createdAt: "now",
                },
              ],
            },
          ]
        : method === "session.get"
          ? {
              id: "ses_test",
              title: "Review <script>bad()</script>",
              sandbox: "standard",
              state: "review",
            }
          : method === "review.fileDiff"
            ? { hunks: [] }
            : {},
  );
  const el = document.createElement("div");
  document.body.append(el);
  cleanups.push(
    render(() => <ReviewPane connection={d.conn} sessionId="ses_test" onClose={() => {}} />, el),
  );
  return {
    ...d,
    el,
    addWithheld() {
      changeset = {
        ...changeset,
        files: [
          ...changeset.files,
          {
            ...changeset.files[0],
            path: "another-large.txt",
            status: "modified",
            additions: 0,
            deletions: 0,
            viewed: false,
            binary: false,
            tooLarge: true,
          },
        ],
      };
      d.emit("checkpoint");
    },
    reorder() {
      changeset = { ...changeset, files: [...changeset.files].reverse() };
      d.emit("checkpoint");
    },
    changeHead() {
      changeset = { ...changeset, head: "cp_new" };
      d.emit("checkpoint");
    },
  };
}
function button(el: HTMLElement, text: string) {
  const b = [...el.querySelectorAll("button")].find((b) => b.textContent === text);
  expect(b, `button ${text}`).toBeDefined();
  if (!b) throw new Error(text);
  return b;
}
it("renders untrusted title, paths, replies and outdated quotes as text", async () => {
  const { el } = mount();
  await settle();
  expect(el.querySelector("img,script")).toBeNull();
  expect(el.textContent).toContain("<img");
  expect(el.textContent).toContain("⟦U+202E⟧");
  expect(el.textContent).toContain("outdated");
});
it("warns that too-large files were not reviewed and requires withheld acknowledgement", async () => {
  const { el, calls } = mount(true);
  await settle();
  expect(el.textContent).toContain("Not reviewed");
  button(el, "Ship").click();
  await settle();
  const submit = button(el, "Push branch");
  expect(submit.disabled).toBe(true);
  expect(calls.some((c) => c.method === "review.ship")).toBe(false);
  const ack = el.querySelector<HTMLInputElement>('input[name="include-withheld"]');
  if (!ack) throw new Error("No acknowledgement");
  ack.click();
  await settle();
  expect(submit.disabled).toBe(false);
  submit.click();
  await settle();
  expect(calls.find((c) => c.method === "review.ship")?.params).toEqual({
    sessionId: "ses_test",
    checkpointId: "cp_reviewed",
    strategy: "push-branch",
    includeWithheld: true,
  });
});
it("sends the reviewed head and clears acknowledgement when reopened", async () => {
  const { el, calls } = mount();
  await settle();
  button(el, "Ship").click();
  await settle();
  button(el, "Push branch").click();
  await settle();
  expect(calls.find((c) => c.method === "review.ship")?.params).toEqual({
    sessionId: "ses_test",
    checkpointId: "cp_reviewed",
    strategy: "push-branch",
  });
});
it("invalidates an open ship confirmation when the checkpoint changes", async () => {
  const { el, calls, changeHead } = mount(true);
  await settle();
  button(el, "Ship").click();
  await settle();
  const ack = el.querySelector<HTMLInputElement>('input[name="include-withheld"]');
  if (!ack) throw new Error("No acknowledgement");
  ack.click();
  changeHead();
  await new Promise((r) => setTimeout(r, 70));
  await settle();
  expect(button(el, "Push branch").disabled).toBe(true);
  expect(ack.checked).toBe(false);
  button(el, "Push branch").click();
  await settle();
  expect(calls.some((c) => c.method === "review.ship")).toBe(false);
});
it("does not treat shortcut keys typed in a composer as review commands", async () => {
  const { el, calls } = mount();
  await settle();
  const input = document.createElement("textarea");
  el.append(input);
  input.dispatchEvent(
    new KeyboardEvent("keydown", { key: "r", ctrlKey: true, shiftKey: true, bubbles: true }),
  );
  await settle();
  expect(calls.some((c) => c.method === "review.requestChanges")).toBe(false);
});
it("expands resolved threads, preserves a refused reply draft and shows daemon details", async () => {
  const { DaemonError } = await import("@forge/client");
  const d = fakeDaemon((method) =>
    method === "review.changeset"
      ? { sessionId: "ses_test", head: "cp", base: "b", round: 1, files: [] }
      : method === "review.threads"
        ? [
            {
              id: "resolved",
              status: "resolved",
              path: "a",
              range: { startLine: 1, endLine: 1 },
              comments: [{ author: "human", body: "previous comment" }],
            },
          ]
        : method === "review.reply"
          ? Promise.reject(
              new DaemonError(-32002, "Refused", {
                kind: "InvalidState",
                detail: "Session no longer accepts replies",
                retryable: false,
              }),
            )
          : {},
  );
  const el = document.createElement("div");
  document.body.append(el);
  cleanups.push(
    render(() => <ReviewPane connection={d.conn} sessionId="ses_test" onClose={() => {}} />, el),
  );
  await settle();
  expect(el.textContent).not.toContain("previous comment");
  button(el, "Show resolved thread").click();
  await settle();
  expect(el.textContent).toContain("previous comment");
  button(el, "Reply").click();
  await settle();
  const input = el.querySelector("textarea");
  if (!input) throw new Error("No reply editor");
  input.value = "keep this draft";
  input.dispatchEvent(new Event("input", { bubbles: true }));
  button(el, "Save reply").click();
  await settle();
  expect(input.value).toBe("keep this draft");
  expect(el.textContent).toContain("Session no longer accepts replies");
});
it("keeps stale content visible and disables shipping on disconnect", async () => {
  const { el, setState } = mount();
  await settle();
  setState({ kind: "reconnecting" });
  await settle();
  expect(el.textContent).toContain("review content is stale");
  expect(button(el, "Ship").disabled).toBe(true);
  expect(el.textContent).toContain("Review <script>");
});
it("shows a failed changeset as a retryable error, without losing threads", async () => {
  const d = fakeDaemon((method) =>
    method === "review.changeset"
      ? Promise.reject(new Error("diff unavailable"))
      : method === "review.threads"
        ? []
        : {},
  );
  const el = document.createElement("div");
  document.body.append(el);
  cleanups.push(
    render(() => <ReviewPane connection={d.conn} sessionId="ses_test" onClose={() => {}} />, el),
  );
  await settle();
  expect(el.textContent).toContain("diff unavailable");
  expect(button(el, "Retry review")).toBeDefined();
  expect(el.textContent).toContain("No comments yet");
});
it("provides keyboard-accessible range selection and preserves a draft on same-head refresh", async () => {
  const { el, setState } = mount();
  await settle();
  const start = el.querySelector<HTMLInputElement>('input[aria-label="Start line"]');
  expect(start).not.toBeNull();
  if (!start) return;
  start.value = "1";
  start.dispatchEvent(new Event("input", { bubbles: true }));
  button(el, "Select lines").click();
  await settle();
  const input = el.querySelector("textarea");
  if (!input) throw new Error("No comment editor");
  input.value = "unsaved comment";
  input.dispatchEvent(new Event("input", { bubbles: true }));
  setState({
    kind: "connected",
    hello: { daemonVersion: "0", apiVersion: "0.5.0", platform: "linux", sandboxTiers: [] },
  });
  await settle();
  expect(el.querySelector("textarea")?.value).toBe("unsaved comment");
});
it("shows invisible path characters but sends their original spelling to setViewed", async () => {
  const path = "isAdmin\u034f.ts";
  const d = fakeDaemon((method) =>
    method === "review.changeset"
      ? {
          sessionId: "ses_test",
          head: "cp",
          base: "b",
          round: 1,
          files: [
            {
              path,
              status: "modified",
              additions: 1,
              deletions: 0,
              binary: false,
              tooLarge: false,
              viewed: false,
            },
          ],
        }
      : method === "review.threads"
        ? []
        : method === "review.fileDiff"
          ? { hunks: [] }
          : {},
  );
  const el = document.createElement("div");
  document.body.append(el);
  cleanups.push(
    render(() => <ReviewPane connection={d.conn} sessionId="ses_test" onClose={() => {}} />, el),
  );
  await settle();
  expect(el.textContent).toContain("isAdmin⟦U+034F⟧.ts");
  el.querySelector<HTMLInputElement>('input[type="checkbox"]')?.click();
  await settle();
  expect(d.calls.find((c) => c.method === "review.setViewed")?.params).toEqual({
    sessionId: "ses_test",
    path,
    viewed: true,
  });
});
it("requires new acknowledgement when the withheld inventory grows at the same head", async () => {
  const { el, calls, addWithheld } = mount(true);
  await settle();
  button(el, "Ship").click();
  await settle();
  const ack = el.querySelector<HTMLInputElement>('input[name="include-withheld"]');
  if (!ack) throw new Error("No acknowledgement");
  ack.click();
  expect(ack.checked).toBe(true);
  addWithheld();
  await new Promise((r) => setTimeout(r, 70));
  await settle();
  expect(ack.checked).toBe(false);
  expect(button(el, "Push branch").disabled).toBe(true);
  button(el, "Push branch").click();
  await settle();
  expect(calls.some((c) => c.method === "review.ship")).toBe(false);
});
it("keeps acknowledgement when the same withheld inventory is only reordered", async () => {
  const { el, addWithheld, reorder } = mount(true);
  await settle();
  addWithheld();
  await new Promise((r) => setTimeout(r, 70));
  button(el, "Ship").click();
  await settle();
  const ack = el.querySelector<HTMLInputElement>('input[name="include-withheld"]');
  if (!ack) throw new Error("No acknowledgement");
  ack.click();
  reorder();
  await new Promise((r) => setTimeout(r, 70));
  await settle();
  expect(ack.checked).toBe(true);
});
it("keeps a large withheld-file acknowledgement list virtualized and readable", async () => {
  const height = vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(400);
  const width = vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(800);
  const d = fakeDaemon((method) =>
    method === "review.changeset"
      ? {
          sessionId: "ses_test",
          head: "cp",
          base: "b",
          round: 1,
          files: Array.from({ length: 150 }, (_, i) => ({
            path: `large-${i}.txt`,
            status: "modified",
            additions: 0,
            deletions: 0,
            binary: false,
            tooLarge: true,
            viewed: false,
          })),
        }
      : method === "review.threads"
        ? []
        : {},
  );
  const el = document.createElement("div");
  document.body.append(el);
  const dispose = render(
    () => <ReviewPane connection={d.conn} sessionId="ses_test" onClose={() => {}} />,
    el,
  );
  try {
    await settle();
    button(el, "Ship").click();
    await settle();
    const dialog = el.querySelector("dialog");
    if (!dialog) throw new Error("No dialog");
    expect(dialog.textContent).toContain("large-0.txt");
    const count = dialog.querySelectorAll("li,.withheld-file").length;
    expect(count).toBeGreaterThan(0);
    expect(count).toBeLessThan(100);
  } finally {
    dispose();
    el.remove();
    height.mockRestore();
    width.mockRestore();
  }
});
