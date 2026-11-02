import { createEffect, createSignal, onCleanup, onMount, Show, Switch, Match } from "solid-js";
import type { LineRange } from "@forge/client";
import type { DaemonConnection } from "../../client/connection";
import { createReviewStore, fileKey } from "../../stores/review";
import { VirtualList } from "../../ui/VirtualList";
import { DiffView } from "./DiffView";
import { Threads } from "./Threads";
import { ShipDialog } from "./ShipDialog";
import { WhyPanel } from "./WhyPanel";
import { visibleText } from "./text";
export function ReviewPane(props: {
  connection: DaemonConnection;
  sessionId: string;
  onClose: () => void;
}) {
  // App keys this component by session ID.
  // eslint-disable-next-line solid/reactivity
  const store = createReviewStore(props.connection, props.sessionId);
  onCleanup(store.dispose);
  const [selection, setSelection] = createSignal<LineRange>(),
    [composing, setComposing] = createSignal(false),
    [body, setBody] = createSignal(""),
    [shipHead, setShipHead] = createSignal<string>(),
    [turn, setTurn] = createSignal<string>(),
    [copyError, setCopyError] = createSignal(""),
    [rangeStart, setRangeStart] = createSignal(1),
    [rangeEnd, setRangeEnd] = createSignal(1),
    [focusedThread, setFocusedThread] = createSignal<string>();
  let composer: HTMLTextAreaElement | undefined;
  const selected = () => store.state.selected;
  const disabled = () => store.state.stale || store.state.busy;
  const openComments = () =>
    store.state.threads
      .filter((t) => t.status !== "resolved")
      .reduce((n, t) => n + t.comments.filter((c) => c.author === "human").length, 0);
  const totals = () =>
    store.state.changeset?.files.reduce(
      (n, f) => ({
        add: n.add + (f.tooLarge ? 0 : f.additions),
        del: n.del + (f.tooLarge ? 0 : f.deletions),
        unknown: n.unknown + (f.tooLarge ? 1 : 0),
      }),
      { add: 0, del: 0, unknown: 0 },
    );
  let previousScope = "";
  createEffect(() => {
    const file = store.state.selected;
    const head = store.state.changeset?.head;
    const from = store.state.fromCheckpoint;
    const scope = JSON.stringify([file?.repo, file?.path, head, from]);
    if (scope === previousScope) return;
    previousScope = scope;
    setSelection(undefined);
    setComposing(false);
    setBody("");
    setTurn(undefined);
  });
  function comment() {
    if (selection() && !disabled()) {
      setComposing(true);
      queueMicrotask(() => composer?.focus());
    }
  }
  async function save() {
    const range = selection();
    if (!range) return;
    try {
      await store.comment(range, body());
      setBody("");
      setComposing(false);
    } catch {
      /* Keep the draft on daemon refusal. */
    }
  }
  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      setCopyError("");
    } catch {
      setCopyError("Could not copy. Clipboard access was refused.");
    }
  }
  function key(e: KeyboardEvent) {
    const target = e.target;
    if (
      e.defaultPrevented ||
      e.isComposing ||
      (target instanceof HTMLElement &&
        target.closest('input,textarea,select,[contenteditable="true"],dialog'))
    )
      return;
    if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key.toLowerCase() === "r") {
      e.preventDefault();
      if (!disabled()) void store.requestChanges().catch(() => {});
      return;
    }
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    const files = store.state.changeset?.files ?? [];
    const index = files.findIndex((f) => fileKey(f) === fileKey(selected() ?? { path: "" }));
    switch (e.key) {
      case "c":
      case "C":
        e.preventDefault();
        comment();
        break;
      case "]":
      case "[": {
        e.preventDefault();
        const next =
          files[Math.max(0, Math.min(files.length - 1, index + (e.key === "]" ? 1 : -1)))];
        if (next) void store.select(next);
        break;
      }
      case "v":
      case "V": {
        const file = selected();
        if (file && !file.tooLarge && store.state.diffStatus !== "withheld" && !disabled()) {
          e.preventDefault();
          void store.setViewed(file, !file.viewed).catch(() => {});
        }
        break;
      }
    }
  }
  onMount(() => window.addEventListener("keydown", key));
  onCleanup(() => window.removeEventListener("keydown", key));
  return (
    <section
      class="review-pane"
      aria-label="Review pane"
      aria-busy={store.state.status === "loading"}
    >
      <header class="review-header">
        <div>
          <button onClick={() => props.onClose()}>Back to projects</button>
          <h2>{visibleText(store.state.session?.title ?? "Session review")}</h2>
          <Show when={store.state.session?.sandbox === "unsandboxed"}>
            <strong class="warning">Unsandboxed</strong>
          </Show>
          <Show when={store.state.changeset}>
            <span>
              {store.state.changeset?.files.length} files · +{totals()?.add} −{totals()?.del} ·
              Round {store.state.changeset?.round}
            </span>
            <Show when={totals()?.unknown}>
              <span class="warning">{totals()?.unknown} files have unknown counts</span>
            </Show>
          </Show>
        </div>
        <div class="review-actions">
          <label>
            Compare
            <select
              value={store.state.sinceLast ? "since" : "full"}
              disabled={disabled() || !store.state.changeset}
              onChange={(e) => void store.setSinceLast(e.currentTarget.value === "since")}
            >
              <option value="full">Full changeset</option>
              <option value="since" disabled={!store.state.changeset?.lastReviewed}>
                Since last review
              </option>
            </select>
          </label>
          <button
            disabled={disabled() || !store.state.changeset}
            onClick={() => void store.requestChanges().catch(() => {})}
          >
            Request changes ({openComments()})
          </button>
          <button
            disabled={disabled() || !store.canShip(store.state.changeset?.head)}
            onClick={() => setShipHead(store.state.changeset?.head)}
          >
            Ship
          </button>
        </div>
      </header>
      <Show when={store.state.stale}>
        <p class="warning" role="status">
          Disconnected · review content is stale. Reconnecting…
        </p>
      </Show>
      <Show when={store.state.actionError}>
        <pre class="warning" role="alert">
          {visibleText(store.state.actionError ?? "")}
        </pre>
      </Show>
      <Show when={copyError()}>
        <p role="alert">{copyError()}</p>
      </Show>
      <Switch>
        <Match when={store.state.status === "loading"}>
          <div class="review-loading skeleton">Loading changeset…</div>
        </Match>
        <Match when={store.state.status === "error"}>
          <p role="alert">{visibleText(store.state.error ?? "Could not load changeset")}</p>
          <button onClick={() => void store.refresh()}>Retry review</button>
        </Match>
        <Match when={store.state.changeset?.files.length === 0}>
          <div class="empty">No changed files in this comparison.</div>
        </Match>
        <Match when={true}>
          <div class="review-layout">
            <nav class="review-files" aria-label="Changed files">
              <VirtualList
                items={store.state.changeset?.files ?? []}
                label="Changed files"
                height="65vh"
                estimate={80}
              >
                {(file) => (
                  <div
                    class="review-file"
                    classList={{
                      selected:
                        !!selected() && fileKey(file) === fileKey(selected() ?? { path: "" }),
                    }}
                  >
                    <label>
                      <input
                        type="checkbox"
                        aria-label={`Viewed ${visibleText(file.path)}`}
                        checked={
                          file.viewed &&
                          !file.tooLarge &&
                          !store.state.withheld.includes(fileKey(file))
                        }
                        disabled={
                          disabled() ||
                          file.tooLarge ||
                          store.state.withheld.includes(fileKey(file))
                        }
                        onChange={(e) =>
                          void store.setViewed(file, e.currentTarget.checked).catch(() => {})
                        }
                      />
                      <span>Viewed</span>
                    </label>
                    <button class="file-path" onClick={() => void store.select(file)}>
                      {visibleText(file.repo ? `${file.repo} / ${file.path}` : file.path)}
                    </button>
                    <span class="hint">
                      {file.tooLarge
                        ? "Not reviewed · too large"
                        : `+${file.additions} −${file.deletions}`}
                    </span>
                  </div>
                )}
              </VirtualList>
            </nav>
            <div class="review-content">
              <Show when={selected()}>
                {(file) => (
                  <>
                    <header class="file-header">
                      <strong>{visibleText(file().path)}</strong>
                      <span class="hint">
                        {file().status}{" "}
                        {file().oldPath ? `from ${visibleText(file().oldPath ?? "")}` : ""} ·{" "}
                        {file().oldMode ?? "—"} → {file().newMode ?? "—"}
                      </span>
                    </header>
                    <Switch>
                      <Match when={store.state.diffStatus === "withheld"}>
                        <div class="warning" role="alert">
                          Not reviewed · this file was withheld from review.{" "}
                          {visibleText(
                            store.state.diffError ?? "It exceeds the review size limit.",
                          )}
                        </div>
                      </Match>
                      <Match when={store.state.diffStatus === "collapsed"}>
                        <div class="empty">
                          Diff starts collapsed for generated, lock, binary or large files.
                          <button
                            disabled={store.state.stale}
                            onClick={() => void store.loadDiff()}
                          >
                            Load diff
                          </button>
                        </div>
                      </Match>
                      <Match when={store.state.diffStatus === "loading"}>
                        <div class="skeleton">Loading file diff…</div>
                      </Match>
                      <Match when={store.state.diffStatus === "error"}>
                        <p role="alert">
                          Not reviewed ·{" "}
                          {visibleText(store.state.diffError ?? "Could not load diff")}
                        </p>
                        <button onClick={() => void store.loadDiff()}>Retry diff</button>
                      </Match>
                      <Match when={store.state.hunks.length === 0}>
                        <p class="empty">No text hunks. Check the file status and modes above.</p>
                      </Match>
                      <Match when={true}>
                        <DiffView
                          store={store}
                          selection={selection()}
                          onThread={setFocusedThread}
                          onSelect={(range) => {
                            setSelection(range);
                            setComposing(true);
                          }}
                          onWhy={setTurn}
                          onCopy={(text) => void copy(text)}
                        />
                      </Match>
                    </Switch>
                  </>
                )}
              </Show>
              <Show when={selected() && store.state.diffStatus === "ready"}>
                <form
                  class="range-selector"
                  aria-label="Comment line range"
                  onSubmit={(e) => {
                    e.preventDefault();
                    if (rangeStart() > 0 && rangeEnd() >= rangeStart()) {
                      setSelection({ startLine: rangeStart(), endLine: rangeEnd() });
                      setComposing(true);
                      queueMicrotask(() => composer?.focus());
                    }
                  }}
                >
                  <label>
                    Start line
                    <input
                      aria-label="Start line"
                      type="number"
                      min="1"
                      required
                      value={rangeStart()}
                      onInput={(e) => setRangeStart(e.currentTarget.valueAsNumber)}
                    />
                  </label>
                  <label>
                    End line
                    <input
                      aria-label="End line"
                      type="number"
                      min={rangeStart()}
                      required
                      value={rangeEnd()}
                      onInput={(e) => setRangeEnd(e.currentTarget.valueAsNumber)}
                    />
                  </label>
                  <button disabled={disabled()}>Select lines</button>
                </form>
              </Show>
              <Show when={selection()}>
                <p class="hint">
                  Selected lines {selection()?.startLine}–{selection()?.endLine}{" "}
                  <button onClick={comment}>Comment (C)</button>
                </p>
              </Show>
              <Show when={composing()}>
                <form
                  class="comment-composer"
                  onSubmit={(e) => {
                    e.preventDefault();
                    void save();
                  }}
                >
                  <label>
                    Comment on selected lines
                    <textarea
                      ref={(el) => (composer = el)}
                      value={body()}
                      onInput={(e) => setBody(e.currentTarget.value)}
                      maxlength={16384}
                    />
                  </label>
                  <button disabled={disabled() || !body().trim()}>Save comment</button>
                  <button type="button" onClick={() => setComposing(false)}>
                    Cancel comment
                  </button>
                  <span class="hint">Saved to Forge; sent to the agent by Request changes.</span>
                </form>
              </Show>
            </div>
            <Show when={turn()} keyed>
              {(id) => (
                <WhyPanel
                  connection={props.connection}
                  sessionId={props.sessionId}
                  turnId={id}
                  onClose={() => setTurn(undefined)}
                />
              )}
            </Show>
          </div>
        </Match>
      </Switch>
      <Threads
        store={store}
        focusedId={focusedThread()}
        onCloseFocus={() => setFocusedThread(undefined)}
      />
      <Show when={shipHead()} keyed>
        {(head) => <ShipDialog store={store} head={head} onClose={() => setShipHead(undefined)} />}
      </Show>
    </section>
  );
}
