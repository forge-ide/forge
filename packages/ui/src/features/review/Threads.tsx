import { createSignal, Show } from "solid-js";
import type { Thread } from "@forge/client";
import type { ReviewStore } from "../../stores/review";
import { VirtualList } from "../../ui/VirtualList";
import { visibleText } from "./text";
export function ThreadCard(props: { thread: Thread; store: ReviewStore }) {
  const [expanded, setExpanded] = createSignal(false),
    [replying, setReplying] = createSignal(false),
    [body, setBody] = createSignal("");
  const send = async () => {
    try {
      await props.store.reply(props.thread.id, body());
      setBody("");
      setReplying(false);
    } catch {
      /* Store retains the daemon refusal; preserve the draft. */
    }
  };
  return (
    <article class="thread" id={`thread-${props.thread.id}`} tabIndex={-1}>
      <header>
        <span>
          {visibleText(props.thread.path)} · {props.thread.range.startLine}–
          {props.thread.range.endLine}
        </span>
        <strong>{props.thread.status}</strong>
      </header>
      <Show
        when={props.thread.status !== "resolved" || expanded()}
        fallback={<button onClick={() => setExpanded(true)}>Show resolved thread</button>}
      >
        <Show when={props.thread.status === "outdated"}>
          <p class="warning">Original code · outdated</p>
          <pre>{visibleText(props.thread.quote.join("\n"))}</pre>
        </Show>
        <VirtualList items={props.thread.comments} label="Thread comments" height="auto">
          {(comment) => (
            <div class="comment">
              <strong>{comment.author === "human" ? "You" : "Agent"}</strong>
              <p>{visibleText(comment.body)}</p>
            </div>
          )}
        </VirtualList>
        <div class="thread-actions">
          <button
            disabled={props.store.state.stale || props.store.state.busy}
            onClick={() => setReplying((v) => !v)}
          >
            Reply
          </button>
          <Show when={props.thread.status !== "resolved"}>
            <button
              disabled={props.store.state.stale || props.store.state.busy}
              onClick={() => void props.store.resolve(props.thread.id).catch(() => {})}
            >
              Resolve
            </button>
          </Show>
          <Show when={props.thread.status === "resolved"}>
            <button onClick={() => setExpanded(false)}>Collapse resolved thread</button>
          </Show>
        </div>
        <Show when={replying()}>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              void send();
            }}
          >
            <label>
              Reply
              <textarea
                value={body()}
                onInput={(e) => setBody(e.currentTarget.value)}
                maxlength={16384}
              />
            </label>
            <button disabled={!body().trim() || props.store.state.stale || props.store.state.busy}>
              Save reply
            </button>
          </form>
        </Show>
      </Show>
    </article>
  );
}
export function Threads(props: {
  store: ReviewStore;
  focusedId: string | undefined;
  onCloseFocus: () => void;
}) {
  return (
    <section class="threads" aria-label="Session review threads">
      <h3>
        Review threads{" "}
        <span class="hint">Saved here until Request changes sends them to the agent</span>
      </h3>
      <Show when={props.store.state.threadsStatus === "loading"}>
        <p>Loading threads…</p>
      </Show>
      <Show when={props.store.state.threadsStatus === "error"}>
        <p role="alert">
          {visibleText(props.store.state.threadsError ?? "Could not load threads")}
        </p>
        <button onClick={() => void props.store.refreshThreads()}>Retry threads</button>
      </Show>
      <Show
        when={props.store.state.threadsStatus === "ready" && props.store.state.threads.length === 0}
      >
        <p class="hint">No comments yet. Select new-side line numbers to start a thread.</p>
      </Show>
      <Show when={props.store.state.threads.find((t) => t.id === props.focusedId)} keyed>
        {(thread) => (
          <section
            class="focused-thread"
            tabIndex={-1}
            ref={(el) =>
              queueMicrotask(() => {
                el.focus();
                el.scrollIntoView?.({ block: "nearest" });
              })
            }
            aria-label="Selected review thread"
          >
            <button onClick={() => props.onCloseFocus()}>Back to all threads</button>
            <ThreadCard thread={thread} store={props.store} />
          </section>
        )}
      </Show>
      <Show when={props.store.state.threads.length > 0}>
        <VirtualList
          items={props.store.state.threads.filter((t) => t.id !== props.focusedId)}
          label="Review threads"
          height="min(50vh, 420px)"
          estimate={180}
        >
          {(thread) => <ThreadCard thread={thread} store={props.store} />}
        </VirtualList>
      </Show>
    </section>
  );
}
