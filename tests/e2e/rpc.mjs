import { connect } from "node:net";
import { WebSocket } from "ws";
export async function connectRpc(socketPath) {
  const ws = new WebSocket("ws://localhost/", {
    createConnection: () => connect(socketPath),
    handshakeTimeout: 5000,
    maxPayload: 32 * 1024 * 1024,
  });
  await new Promise((resolve, reject) => {
    ws.once("open", resolve);
    ws.once("error", reject);
  });
  let next = 0;
  const pending = new Map();
  function fail(error) {
    for (const { reject, timer } of pending.values()) {
      clearTimeout(timer);
      reject(error);
    }
    pending.clear();
  }
  ws.on("error", fail);
  ws.on("close", () => fail(new Error("Fixture RPC connection closed")));
  ws.on("message", (data) => {
    let message;
    try {
      message = JSON.parse(data.toString());
    } catch {
      return;
    }
    const request = pending.get(message.id);
    if (!request) return;
    pending.delete(message.id);
    clearTimeout(request.timer);
    if (message.error)
      request.reject(new Error(`${message.error.message}: ${message.error.data?.detail ?? ""}`));
    else request.resolve(message.result);
  });
  return {
    call(method, params) {
      return new Promise((resolve, reject) => {
        const id = ++next;
        const timer = setTimeout(() => {
          pending.delete(id);
          reject(new Error(`Fixture RPC timed out: ${method}`));
        }, 15000);
        pending.set(id, { resolve, reject, timer });
        ws.send(JSON.stringify({ jsonrpc: "2.0", id, method, params }), (error) => {
          if (error) {
            pending.delete(id);
            clearTimeout(timer);
            reject(error);
          }
        });
      });
    },
    async close() {
      fail(new Error("Fixture RPC shutdown"));
      if (ws.readyState === WebSocket.CLOSED) return;
      await new Promise((resolve) => {
        ws.once("close", resolve);
        ws.close();
        setTimeout(() => ws.terminate(), 1000).unref();
      });
    },
  };
}
export async function until(
  read,
  predicate,
  { timeout = 15000, label = "condition", signal } = {},
) {
  const deadline = Date.now() + timeout;
  let lastError;
  while (Date.now() < deadline) {
    signal?.throwIfAborted();
    try {
      const value = await read();
      if (predicate(value)) return value;
    } catch (error) {
      lastError = error;
    }
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  throw new Error(`Timed out waiting for ${label}${lastError ? `: ${lastError.message}` : ""}`);
}
