import { render } from "solid-js/web";
import { App } from "./App";
import { DaemonConnection } from "./client/connection";
import { createTauriTransport } from "./host/daemon";
import "./styles.css";

const root = document.getElementById("root");
if (!root) {
  throw new Error("missing #root element");
}
const connection = new DaemonConnection(createTauriTransport());
connection.start();
render(() => <App connection={connection} />, root);
