import { For, Match, Switch } from "solid-js";
import { VirtualList } from "../../ui/VirtualList";
import { SessionList } from "./SessionList";
import type { DaemonConnection } from "../../client/connection";
import { visibleText } from "../review/text";
import type { ProjectsState } from "../../stores/projects";

/** The welcome view's project list, with loading, empty, error, and stale states. */
export function ProjectList(props: {
  state: ProjectsState;
  onRetry: () => void;
  connection: DaemonConnection;
  onSelectSession: (id: string) => void;
}) {
  return (
    <section
      class="projects"
      classList={{ stale: props.state.stale }}
      aria-busy={props.state.status === "loading"}
    >
      <h2>Projects</h2>
      <Switch>
        <Match when={props.state.status === "loading"}>
          <ul class="project-list" aria-label="Loading projects">
            <For each={[0, 1, 2]}>{() => <li class="skeleton" />}</For>
          </ul>
        </Match>
        <Match when={props.state.status === "error"}>
          <div class="empty" role="alert">
            <p>Couldn't load projects: {props.state.error}</p>
            <button type="button" onClick={() => props.onRetry()}>
              Retry
            </button>
          </div>
        </Match>
        <Match when={props.state.projects.length === 0}>
          <div class="empty">
            <p>No projects yet.</p>
            <p class="hint">
              Open one from a terminal with <code>forge project open &lt;path&gt;</code>.
            </p>
          </div>
        </Match>
        <Match when={true}>
          <div class="project-list">
            <VirtualList items={props.state.projects} label="Projects" height="70vh" estimate={200}>
              {(project) => (
                <div class="project">
                  <span class="project-name">{visibleText(project.name)}</span>
                  <span class="project-root">{visibleText(project.root)}</span>
                  <span class="project-repos">
                    <For each={project.repos}>
                      {(repo) => (
                        <span class="repo">
                          {visibleText(repo.name)} · {visibleText(repo.baseBranch)}
                        </span>
                      )}
                    </For>
                  </span>
                  <SessionList
                    connection={props.connection}
                    projectId={project.id}
                    onSelect={props.onSelectSession}
                  />
                </div>
              )}
            </VirtualList>
          </div>
        </Match>
      </Switch>
    </section>
  );
}
