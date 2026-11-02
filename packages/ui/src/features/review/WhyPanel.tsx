import { onCleanup, Show, Switch, Match } from "solid-js";
import type { DaemonConnection } from "../../client/connection";
import { createTranscriptStore } from "../../stores/transcript";
import { VirtualList } from "../../ui/VirtualList";
import { visibleText } from "./text";
export function WhyPanel(props: {
  connection: DaemonConnection;
  sessionId: string;
  turnId: string;
  onClose: () => void;
}) {
  // Keyed by turn ID in ReviewPane.
  // eslint-disable-next-line solid/reactivity
  const store = createTranscriptStore(props.connection, props.sessionId, props.turnId);
  onCleanup(store.dispose);
  return (
    <aside class="why-panel" aria-label="Why this hunk">
      <header>
        <h3>Why this hunk</h3>
        <button onClick={() => props.onClose()}>Close why</button>
      </header>
      <code>{visibleText(props.turnId)}</code>
      <Show when={store.state.stale}>
        <p class="warning">Disconnected · transcript may be stale</p>
      </Show>
      <Switch>
        <Match when={store.state.status === "loading"}>
          <p>Loading transcript turn… {store.state.pages} pages read</p>
        </Match>
        <Match when={store.state.status === "error"}>
          <p role="alert">{visibleText(store.state.error ?? "Could not load turn")}</p>
          <button onClick={() => void store.refresh()}>Retry transcript</button>
        </Match>
        <Match when={true}>
          <Show when={store.state.partial}>
            <p class="warning">Incomplete history: turn boundaries are missing.</p>
          </Show>
          <Show when={store.state.rows.length === 0}>
            <p>No transcript content available for this turn.</p>
          </Show>
          <VirtualList
            items={store.state.rows}
            label="Originating transcript turn"
            height="65vh"
            estimate={120}
          >
            {(row) => (
              <article class="transcript-row">
                <Show
                  when={row.detail}
                  fallback={
                    <>
                      <strong>{row.label}</strong>
                      <pre>{visibleText(row.text)}</pre>
                    </>
                  }
                >
                  <details>
                    <summary>{row.label}</summary>
                    <pre>{visibleText(row.text)}</pre>
                  </details>
                </Show>
              </article>
            )}
          </VirtualList>
        </Match>
      </Switch>
    </aside>
  );
}
