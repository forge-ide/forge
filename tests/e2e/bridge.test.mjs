import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, mkdir, writeFile, rm } from "node:fs/promises";
import { createServer } from "node:http";
import { once } from "node:events";
import { join } from "node:path";
import { WebSocket, WebSocketServer } from "ws";
import { startBridge } from "./bridge.mjs";
async function fixture() {
  const root = await mkdtemp("/tmp/forge-bridge-");
  const assets = join(root, "assets");
  await mkdir(assets);
  await writeFile(join(assets, "index.html"), "<!doctype html><p>fixture</p>");
  const socket = join(root, "daemon.sock");
  const server = createServer();
  const ws = new WebSocketServer({ server });
  ws.on("connection", (client) => client.on("message", (data) => client.send(data.toString())));
  server.listen(socket);
  await once(server, "listening");
  let bridge;
  try {
    bridge = await startBridge({ socketPath: socket, assetRoot: assets, token: "a".repeat(64) });
  } catch (e) {
    server.close();
    await rm(root, { recursive: true, force: true });
    throw e;
  }
  return {
    bridge,
    async close() {
      await bridge.close();
      for (const client of ws.clients) client.terminate();
      await new Promise((resolve) => ws.close(resolve));
      await new Promise((resolve) => server.close(resolve));
      await rm(root, { recursive: true, force: true });
    },
  };
}
function connectWebSocket(url, origin) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(url, { origin });
    ws.once("open", () => resolve(ws));
    ws.once("error", reject);
  });
}
test("forwards text through the fixed Unix socket and serves only its assets", async () => {
  const f = await fixture();
  try {
    const page = new URL(f.bridge.url);
    const ws = await connectWebSocket(f.bridge.wsUrl, page.origin);
    ws.send("hello");
    const [reply] = await once(ws, "message");
    assert.equal(reply.toString(), "hello");
    ws.close();
    await once(ws, "close");
    assert.match(await (await fetch(f.bridge.url)).text(), /fixture/);
    assert.equal((await fetch(`${page.origin}/%2e%2e%2foutside`)).status, 404);
  } finally {
    await f.close();
  }
});
test("rejects wrong or absent Origin, wrong token and arbitrary bridge paths", async () => {
  const f = await fixture();
  try {
    const origin = new URL(f.bridge.url).origin;
    await assert.rejects(connectWebSocket(f.bridge.wsUrl, "http://evil.example"));
    await assert.rejects(connectWebSocket(f.bridge.wsUrl, undefined));
    await assert.rejects(
      connectWebSocket(f.bridge.wsUrl.replace("a".repeat(64), "b".repeat(64)), origin),
    );
    await assert.rejects(connectWebSocket(f.bridge.wsUrl.replace("/bridge/", "/other/"), origin));
  } finally {
    await f.close();
  }
});
test("bounds frames and shuts down active websocket clients", async () => {
  const f = await fixture();
  try {
    const ws = await connectWebSocket(f.bridge.wsUrl, new URL(f.bridge.url).origin);
    const closed = once(ws, "close");
    ws.send("x".repeat(1024 * 1024 + 1));
    await closed;
    const active = await connectWebSocket(f.bridge.wsUrl, new URL(f.bridge.url).origin);
    const ended = once(active, "close");
    await f.bridge.close();
    await ended;
  } finally {
    await f.close();
  }
});
test("shutdown closes unauthenticated idle and partial-header TCP connections as well as active WebSockets", async () => {
  const { connect } = await import("node:net");
  const f = await fixture();
  const clients = [];
  let closing;
  try {
    const page = new URL(f.bridge.url);
    for (const partial of [false, true]) {
      const client = connect({ host: "127.0.0.1", port: Number(page.port) });
      clients.push(client);
      await once(client, "connect");
      if (partial) client.write(`GET / HTTP/1.1\r\nHost: ${page.host}\r\n`);
    }
    const ws = await connectWebSocket(f.bridge.wsUrl, page.origin);
    clients.push(ws);
    const ended = clients.map((client) => new Promise((resolve) => client.once("close", resolve)));
    closing = Promise.all([f.bridge.close(), f.bridge.close()]);
    let timer;
    try {
      await Promise.race([
        closing,
        new Promise((_, reject) => {
          timer = setTimeout(
            () => reject(new Error("Bridge shutdown left raw TCP sockets open")),
            500,
          );
        }),
      ]);
    } finally {
      clearTimeout(timer);
    }
    await Promise.all(ended);
  } finally {
    for (const client of clients) {
      if ("terminate" in client) client.terminate();
      else client.destroy();
    }
    await closing?.catch(() => {});
    await f.close();
  }
});
