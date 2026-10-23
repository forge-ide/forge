// The daemon connection state as a Solid signal.

import { createSignal, type Accessor } from "solid-js";
import type { ConnectionState, DaemonConnection } from "../client/connection";

export function createConnectionState(conn: DaemonConnection): Accessor<ConnectionState> {
  const [state, setState] = createSignal<ConnectionState>(conn.state);
  conn.onStateChange(setState);
  return state;
}
