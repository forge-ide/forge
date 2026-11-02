import { createEffect, onCleanup, untrack } from "solid-js";
import { mountDiff } from "@forge/editor";
import type { LineRange } from "@forge/client";
import type { ReviewStore } from "../../stores/review";
import { visibleText } from "./text";
export function DiffView(props: {
  store: ReviewStore;
  selection: LineRange | undefined;
  onThread: (id: string) => void;
  onSelect: (range: LineRange) => void;
  onWhy: (turn: string) => void;
  onCopy: (text: string) => void;
}) {
  let parent: HTMLDivElement | undefined;
  let current: ReturnType<typeof mountDiff> | undefined;
  createEffect(() => {
    const hunks = props.store.state.hunks;
    const threads = props.store.state.threads;
    const file = props.store.state.selected;
    const disabled = props.store.state.stale || props.store.state.busy;
    const host = parent;
    if (!host) return;
    const view = untrack(() =>
      mountDiff(host, hunks, {
        disabled,
        onSelect: (range) => props.onSelect(range),
        onWhy: (turn) => props.onWhy(turn),
        onCopy: (text) => props.onCopy(text),
        onRevert: (id) => void props.store.revert(id).catch(() => {}),
        widgets: threads
          .filter(
            (t) =>
              t.status !== "resolved" &&
              t.path === file?.path &&
              (t.repo ?? "") === (file?.repo ?? "") &&
              t.current,
          )
          .map((thread) => ({
            line: thread.current?.endLine ?? 0,
            create() {
              const el = document.createElement("aside");
              el.className = "inline-thread";
              const button = document.createElement("button");
              button.textContent = `${thread.comments.length} comments · ${thread.status}`;
              button.addEventListener("click", () => props.onThread(thread.id));
              el.append(button);
              for (const comment of thread.comments.slice(-2)) {
                const p = document.createElement("p");
                p.textContent = visibleText(`${comment.author}: ${comment.body}`);
                el.append(p);
              }
              return el;
            },
          })),
      }),
    );
    current = view;
    view.setSelection(untrack(() => props.selection));
    onCleanup(() => {
      view.destroy();
      current = undefined;
    });
  });
  createEffect(() => {
    current?.setSelection(props.selection);
  });
  return <div class="diff-editor" ref={(el) => (parent = el)} />;
}
