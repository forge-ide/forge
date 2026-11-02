import { createSignal, Show, onMount, createEffect } from "solid-js";
import type { ReviewStore } from "../../stores/review";
import { fileKey } from "../../stores/review";
import { VirtualList } from "../../ui/VirtualList";
import { visibleText } from "./text";
export function ShipDialog(props: { store: ReviewStore; head: string; onClose: () => void }) {
  const [ackKey, setAckKey] = createSignal<string>(),
    [branch, setBranch] = createSignal(""),
    [result, setResult] = createSignal("");
  let dialog: HTMLDialogElement | undefined;
  const withheld = () =>
    props.store.state.fullChangeset?.files.filter(
      (f) =>
        f.tooLarge ||
        f.additions + f.deletions > 200000 ||
        props.store.state.withheld.includes(fileKey(f)),
    ) ?? [];
  const consentKey = () =>
    JSON.stringify([
      props.head,
      props.store.state.fromCheckpoint ?? null,
      withheld().map(fileKey).sort(),
    ]);
  const ack = () => ackKey() === consentKey();
  let previousConsentKey = "";
  createEffect(() => {
    const key = consentKey();
    if (key !== previousConsentKey) {
      previousConsentKey = key;
      setAckKey(undefined);
    }
  });
  const outdated = () => props.head !== props.store.state.changeset?.head;
  const disabled = () =>
    props.store.state.busy ||
    props.store.state.stale ||
    outdated() ||
    !props.store.canShip(props.head) ||
    (withheld().length > 0 && !ack());
  async function ship(strategy: "push-branch" | "merge-local") {
    if (disabled()) return;
    try {
      const result = await props.store.ship(
        props.head,
        strategy,
        branch(),
        ack() && withheld().length > 0,
      );
      setResult(result.ref);
    } catch {
      /* Keep refusal and acknowledgement visible. */
    }
  }
  createEffect(() => {
    if (outdated()) setAckKey(undefined);
  });
  onMount(() => {
    if (dialog?.showModal) dialog.showModal();
    else if (dialog) dialog.open = true;
  });
  return (
    <dialog
      ref={(el) => (dialog = el)}
      class="ship-dialog"
      aria-label="Ship reviewed checkpoint"
      onCancel={(e) => {
        e.preventDefault();
        props.onClose();
      }}
    >
      <h2>Ship reviewed checkpoint</h2>
      <code>{visibleText(props.head)}</code>
      <Show when={outdated()}>
        <p role="alert">The changeset changed. Close and review it before shipping.</p>
      </Show>
      <Show when={withheld().length > 0}>
        <p class="warning">These files were withheld and are not reviewed:</p>
        <VirtualList items={withheld()} label="Withheld files" height="auto" estimate={36}>
          {(file) => (
            <p class="withheld-file">
              {visibleText(file.repo ? `${file.repo} / ${file.path}` : file.path)}
            </p>
          )}
        </VirtualList>
        <label class="acknowledgement">
          <input
            type="checkbox"
            name="include-withheld"
            checked={ack()}
            onChange={(e) => setAckKey(e.currentTarget.checked ? consentKey() : undefined)}
          />{" "}
          I acknowledge these files were not reviewed and want to include them.
        </label>
      </Show>
      <Show when={props.store.state.actionError}>
        <pre class="warning" role="alert">
          {visibleText(props.store.state.actionError ?? "")}
        </pre>
      </Show>
      <label>
        Remote branch <span class="hint">Optional; push only</span>
        <input
          value={branch()}
          onInput={(e) => setBranch(e.currentTarget.value)}
          placeholder="Session branch"
        />
      </label>
      <p class="hint">
        Push publishes the session branch. Merge locally updates the base branch and finishes the
        session.
      </p>
      <Show when={result()}>
        <p role="status">Shipped to {visibleText(result())}</p>
      </Show>
      <div class="dialog-actions">
        <button onClick={() => props.onClose()}>Close</button>
        <button disabled={disabled()} onClick={() => void ship("merge-local")}>
          Merge locally
        </button>
        <button disabled={disabled()} onClick={() => void ship("push-branch")}>
          Push branch
        </button>
      </div>
    </dialog>
  );
}
