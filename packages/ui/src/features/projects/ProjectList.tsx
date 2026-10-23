import { For, Match, Switch } from "solid-js";
import type { ProjectsState } from "../../stores/projects";

/** The welcome view's project list, with loading, empty, error, and stale states. */
export function ProjectList(props: { state: ProjectsState; onRetry: () => void }) {
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
          <ul class="project-list">
            <For each={props.state.projects}>
              {(project) => (
                <li class="project">
                  <span class="project-name">{project.name}</span>
                  <span class="project-root">{project.root}</span>
                  <span class="project-repos">
                    <For each={project.repos}>
                      {(repo) => (
                        <span class="repo">
                          {repo.name} · {repo.baseBranch}
                        </span>
                      )}
                    </For>
                  </span>
                </li>
              )}
            </For>
          </ul>
        </Match>
      </Switch>
    </section>
  );
}
