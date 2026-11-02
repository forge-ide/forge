import { expect, it } from "vitest";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { fakeDaemon, settle } from "../../testing/daemon";
import { createReviewStore } from "../../stores/review";
import { Threads } from "./Threads";
it("opens a focused thread even when it is outside the virtualized viewport", async () => {
  const d = fakeDaemon((method) =>
    method === "review.threads"
      ? Array.from({ length: 150 }, (_, i) => ({
          id: `thread_${i}`,
          path: "a",
          status: "open",
          range: { startLine: 1, endLine: 1 },
          comments: [{ author: "human", body: `comment_${i}` }],
        }))
      : method === "review.changeset"
        ? { head: "cp", files: [] }
        : {},
  );
  const store = createReviewStore(d.conn, "ses_test");
  const [id, setId] = createSignal<string>();
  const parent = document.createElement("div");
  document.body.append(parent);
  const dispose = render(
    () => <Threads store={store} focusedId={id()} onCloseFocus={() => setId(undefined)} />,
    parent,
  );
  await settle();
  expect(parent.textContent).not.toContain("comment_149");
  setId("thread_149");
  await settle();
  expect(parent.textContent).toContain("comment_149");
  dispose();
  store.dispose();
  parent.remove();
});
