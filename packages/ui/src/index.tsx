import { render } from "solid-js/web";
import { App } from "./App";
import { DaemonConnection } from "./client/connection";
import { createTransport } from "./host/platform";
import "./styles.css";

const root = document.getElementById("root");
if (!root) {
  throw new Error("missing #root element");
}
const connection = new DaemonConnection(createTransport());
connection.start();
render(() => <App connection={connection} />, root);
