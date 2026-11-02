import { afterEach, expect, it, vi } from "vitest";
import { createWebTransport } from "./web";
class Socket extends EventTarget {
  static OPEN = 1;
  static all: Socket[] = [];
  readyState = 0;
  frames: string[] = [];
  constructor(readonly url: string) {
    super();
    Socket.all.push(this);
  }
  send(text: string) {
    this.frames.push(text);
  }
  close() {
    this.readyState = 3;
    this.dispatchEvent(new Event("close"));
  }
  open() {
    this.readyState = 1;
    this.dispatchEvent(new Event("open"));
  }
}
afterEach(() => {
  vi.unstubAllGlobals();
  Socket.all = [];
});
const pageUrl = `http://127.0.0.1:1234/?bridge=${"a".repeat(64)}`;
it("connects only to the page loopback bridge, forwards text and reports close", async () => {
  vi.stubGlobal("WebSocket", Socket);
  const t = createWebTransport({ pageUrl });
  const received: string[] = [];
  let closed = 0;
  t.onMessage((text) => received.push(text));
  t.onClose(() => closed++);
  await expect(t.send("early")).rejects.toThrow();
  const ready = t.connect();
  const socket = Socket.all[0];
  expect(socket?.url).toBe(`ws://127.0.0.1:1234/bridge/${"a".repeat(64)}`);
  socket?.open();
  await ready;
  await t.send("request");
  expect(socket?.frames).toEqual(["request"]);
  socket?.dispatchEvent(new MessageEvent("message", { data: "response" }));
  expect(received).toEqual(["response"]);
  socket?.close();
  expect(closed).toBe(1);
});
it("rejects missing capability or a non-loopback page", async () => {
  expect(() => createWebTransport({ pageUrl: "http://127.0.0.1:1234/" })).toThrow();
  expect(() =>
    createWebTransport({ pageUrl: pageUrl.replace("127.0.0.1", "example.com") }),
  ).toThrow();
});
it("replacement closes the old socket without reporting it as the new connection closing", async () => {
  vi.stubGlobal("WebSocket", Socket);
  const t = createWebTransport({ pageUrl });
  let closed = 0;
  t.onClose(() => closed++);
  const first = t.connect();
  Socket.all[0]?.open();
  await first;
  const next = t.connect();
  Socket.all[1]?.open();
  await next;
  expect(Socket.all[0]?.readyState).toBe(3);
  expect(closed).toBe(0);
  Socket.all[1]?.close();
  expect(closed).toBe(1);
});
