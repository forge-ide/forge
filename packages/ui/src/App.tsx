import { Show, createSignal } from "solid-js";
import type { DaemonConnection } from "./client/connection";
import { ReviewPane } from "./features/review/ReviewPane";
import { ProjectList } from "./features/projects/ProjectList";
import { createConnectionState } from "./stores/connection";
import { createProjectsStore } from "./stores/projects";
import { appTitle } from "./title";
import { ConnectionBanner } from "./ui/ConnectionBanner";

export function App(props: { connection: DaemonConnection }) {
  // The connection is created once at startup and never replaced.
  // eslint-disable-next-line solid/reactivity
  const conn = props.connection;
  const connection = createConnectionState(conn);
  const projects = createProjectsStore(conn);
  const [selectedSession, setSelectedSession] = createSignal<string>();

  const status = () => {
    const s = connection();
    switch (s.kind) {
      case "connected":
        return `Connected · forged ${s.hello.daemonVersion}`;
      case "reconnecting":
        return "Disconnected";
      case "failed":
        return "Not connected";
      default:
        return "Connecting";
    }
  };

  return (
    <>
      <ConnectionBanner state={connection} onRetry={() => conn.retry()} />
      <header class="app-header">
        <h1>{appTitle()}</h1>
        <span class="status" classList={{ ok: connection().kind === "connected" }}>
          {status()}
        </span>
      </header>
      <main classList={{ "review-open": !!selectedSession() }}>
        <Show when={connection().kind !== "starting"}>
          <Show
            when={selectedSession()}
            keyed
            fallback={
              <ProjectList
                connection={conn}
                onSelectSession={setSelectedSession}
                state={projects.state}
                onRetry={() => void projects.refresh()}
              />
            }
          >
            {(id) => (
              <ReviewPane
                connection={conn}
                sessionId={id}
                onClose={() => setSelectedSession(undefined)}
              />
            )}
          </Show>
        </Show>
      </main>
    </>
  );
}
