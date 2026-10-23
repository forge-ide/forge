import { Match, Show, Switch, type Accessor } from "solid-js";
import type { ConnectionState } from "../client/connection";

/**
 * Start-up and failure cover the window (ui-screens.md, "Daemon not reachable");
 * a drop after connecting shows a thin banner and leaves content visible.
 */
export function ConnectionBanner(props: { state: Accessor<ConnectionState>; onRetry: () => void }) {
  return (
    <Switch>
      <Match when={props.state().kind === "starting"}>
        <div class="cover" role="status">
          <p class="cover-title">Starting Forge daemon...</p>
        </div>
      </Match>
      <Match when={narrowFailed(props.state())}>
        {(failed) => (
          <div class="cover" role="alert">
            <p class="cover-title">Can't reach the Forge daemon</p>
            <p class="cover-detail">{failed().error}</p>
            <Show when={failed().logPath}>
              {(log) => (
                <p class="cover-detail">
                  Daemon log: <code>{log()}</code>
                </p>
              )}
            </Show>
            <button type="button" onClick={() => props.onRetry()}>
              Retry
            </button>
          </div>
        )}
      </Match>
      <Match when={props.state().kind === "reconnecting"}>
        <div class="banner" role="status">
          Reconnecting...
        </div>
      </Match>
    </Switch>
  );
}

function narrowFailed(state: ConnectionState) {
  return state.kind === "failed" ? state : undefined;
}
