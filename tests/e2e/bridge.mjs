// Test-only loopback bridge. forged remains exclusively on its private Unix socket.
import { createServer } from "node:http";
import { connect as connectUnix } from "node:net";
import { readFile, realpath, stat } from "node:fs/promises";
import { resolve, sep, extname } from "node:path";
import { randomBytes, timingSafeEqual } from "node:crypto";
import { once } from "node:events";
import { WebSocket, WebSocketServer } from "ws";
export async function startBridge({
  socketPath,
  assetRoot,
  token = randomBytes(32).toString("hex"),
}) {
  if (!/^[a-f0-9]{64}$/.test(token)) throw new Error("Invalid bridge capability");
  const root = await realpath(assetRoot);
  const peers = new Set();
  const sockets = new Set();
  let closing;
  let origin,
    closed = false;
  const server = createServer(async (req, res) => {
    try {
      if (req.headers.host !== new URL(origin).host || !["GET", "HEAD"].includes(req.method)) {
        res.writeHead(403).end();
        return;
      }
      const path = decodeURIComponent(new URL(req.url, origin).pathname);
      const file = await realpath(resolve(root, `.${path === "/" ? "/index.html" : path}`));
      if (!file.startsWith(root + sep) || !(await stat(file)).isFile()) {
        res.writeHead(404).end();
        return;
      }
      const type = {
        ".html": "text/html",
        ".js": "text/javascript",
        ".css": "text/css",
        ".svg": "image/svg+xml",
        ".png": "image/png",
      }[extname(file)];
      if (!type) {
        res.writeHead(404).end();
        return;
      }
      const data = await readFile(file);
      res.writeHead(200, {
        "Content-Type": type,
        "Cache-Control": "no-store",
        "X-Content-Type-Options": "nosniff",
        "Content-Security-Policy": `default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src ${origin.replace("http:", "ws:")}`,
      });
      res.end(req.method === "HEAD" ? undefined : data);
    } catch {
      res.writeHead(404).end();
    }
  });
  server.headersTimeout = 5000;
  server.requestTimeout = 5000;
  server.keepAliveTimeout = 1000;
  server.on("connection", (socket) => {
    if (closed) {
      socket.destroy();
      return;
    }
    sockets.add(socket);
    socket.setTimeout(5000, () => socket.destroy());
    socket.once("close", () => sockets.delete(socket));
  });
  const wss = new WebSocketServer({ noServer: true, maxPayload: 1024 * 1024 });
  server.on("upgrade", (req, socket, head) => {
    const match = /^\/bridge\/([a-f0-9]{64})$/.exec(req.url ?? "");
    if (
      closed ||
      req.headers.host !== new URL(origin).host ||
      req.headers.origin !== origin ||
      !match ||
      !timingSafeEqual(Buffer.from(match[1]), Buffer.from(token))
    ) {
      socket.end("HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n");
      return;
    }
    socket.setTimeout(0);
    wss.handleUpgrade(req, socket, head, (client) => {
      const upstream = new WebSocket("ws://localhost/", {
        createConnection: () => connectUnix(socketPath),
        maxPayload: 1024 * 1024,
        handshakeTimeout: 5000,
      });
      peers.add(client);
      peers.add(upstream);
      let pending = [],
        bytes = 0;
      client.on("error", () => {});
      upstream.on("error", () => client.close(1011, "Fixture daemon unavailable"));
      client.on("message", (data, binary) => {
        if (binary) {
          client.close(1003);
          return;
        }
        if (upstream.readyState === WebSocket.OPEN) upstream.send(data.toString());
        else if (upstream.readyState === WebSocket.CONNECTING) {
          bytes += data.length;
          if (bytes > 1024 * 1024) {
            client.close(1009);
            return;
          }
          pending.push(data.toString());
        }
      });
      upstream.once("open", () => {
        if (client.readyState !== WebSocket.OPEN) {
          upstream.close();
          return;
        }
        for (const text of pending) upstream.send(text);
        pending = [];
        bytes = 0;
      });
      upstream.on("message", (data, binary) => {
        if (client.readyState === WebSocket.OPEN && !binary) client.send(data.toString());
      });
      client.once("close", () => {
        peers.delete(client);
        upstream.close();
      });
      upstream.once("close", () => {
        peers.delete(upstream);
        client.close();
      });
    });
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const address = server.address();
  origin = `http://127.0.0.1:${address.port}`;
  return {
    url: `${origin}/?bridge=${token}`,
    wsUrl: `${origin}/bridge/${token}`,
    close() {
      if (closing) return closing;
      closed = true;
      const httpClosed = new Promise((resolve) => server.close(resolve));
      closing = (async () => {
        const wsClosed = new Promise((resolve) => wss.close(resolve));
        for (const peer of peers) peer.terminate();
        peers.clear();
        for (const socket of sockets) socket.destroy();
        sockets.clear();
        await Promise.all([httpClosed, wsClosed]);
      })();
      return closing;
    },
  };
}
