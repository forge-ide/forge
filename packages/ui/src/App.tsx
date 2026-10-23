import { Show } from "solid-js";
import type { DaemonConnection } from "./client/connection";
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
      <main>
        <Show when={connection().kind !== "starting"}>
          <ProjectList state={projects.state} onRetry={() => void projects.refresh()} />
        </Show>
      </main>
    </>
  );
}
