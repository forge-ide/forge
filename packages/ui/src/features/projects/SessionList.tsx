import { onCleanup, Switch, Match, Show } from "solid-js";
import type { DaemonConnection } from "../../client/connection";
import { createSessionsStore } from "../../stores/sessions";
import { visibleText } from "../review/text";
import { VirtualList } from "../../ui/VirtualList";
export function SessionList(props: {
  connection: DaemonConnection;
  projectId: string;
  onSelect: (id: string) => void;
}) {
  // ProjectList keys this component by project ID.
  // eslint-disable-next-line solid/reactivity
  const store = createSessionsStore(props.connection, props.projectId);
  onCleanup(store.dispose);
  return (
    <section
      class="session-list"
      aria-label="Project sessions"
      aria-busy={store.state.status === "loading"}
    >
      <Show when={store.state.stale}>
        <p class="warning">Disconnected · sessions may be stale</p>
      </Show>
      <Switch>
        <Match when={store.state.status === "loading"}>
          <p class="skeleton">Loading sessions…</p>
        </Match>
        <Match when={store.state.status === "error"}>
          <p role="alert">{visibleText(store.state.error ?? "Could not load sessions")}</p>
          <button onClick={() => void store.refresh()}>Retry sessions</button>
        </Match>
        <Match when={store.state.sessions.length === 0}>
          <p class="hint">
            No sessions yet. Create a session with the Forge CLI to review its changes.
          </p>
        </Match>
        <Match when={true}>
          <VirtualList items={store.state.sessions} label="Sessions" height="min(30vh, 220px)">
            {(session) => (
              <button class="session-row" onClick={() => props.onSelect(session.id)}>
                <span>{visibleText(session.title)}</span>
                <span class="hint">
                  {session.state} · {visibleText(session.branch)}
                </span>
              </button>
            )}
          </VirtualList>
        </Match>
      </Switch>
    </section>
  );
}
