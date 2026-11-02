import type { DaemonTransport } from "./transport";
/** Browser-test host only. The ordinary desktop build never imports this module. */
export function createWebTransport(options: { pageUrl?: string } = {}): DaemonTransport {
  const page = new URL(options.pageUrl ?? window.location.href);
  const token = page.searchParams.get("bridge");
  if (
    page.protocol !== "http:" ||
    page.hostname !== "127.0.0.1" ||
    !page.port ||
    !token ||
    !/^[a-f0-9]{64}$/.test(token)
  )
    throw new Error("Expected a capability-bearing loopback test page");
  const url = `ws://${page.host}/bridge/${token}`;
  let socket: WebSocket | undefined;
  const messages = new Set<(text: string) => void>(),
    closes = new Set<(reason: string) => void>();
  return {
    connect() {
      const old = socket;
      socket = undefined;
      old?.close();
      return new Promise<void>((resolve, reject) => {
        const next = new WebSocket(url);
        socket = next;
        let opened = false;
        next.addEventListener("open", () => {
          if (socket !== next) {
            next.close();
            reject(new Error("Connection replaced"));
            return;
          }
          opened = true;
          resolve();
        });
        next.addEventListener("message", (event) => {
          if (socket === next && typeof event.data === "string")
            messages.forEach((fn) => fn(event.data));
        });
        next.addEventListener("error", () => {
          if (!opened) reject(new Error("Could not connect to test bridge"));
        });
        next.addEventListener("close", () => {
          if (!opened) reject(new Error("Test bridge closed before connecting"));
          if (socket === next) {
            socket = undefined;
            closes.forEach((fn) => fn("Test bridge connection closed"));
          }
        });
      });
    },
    async send(text) {
      if (socket?.readyState !== WebSocket.OPEN) throw new Error("Not connected to test bridge");
      socket.send(text);
    },
    onMessage(fn) {
      messages.add(fn);
      return () => {
        messages.delete(fn);
      };
    },
    onClose(fn) {
      closes.add(fn);
      return () => {
        closes.delete(fn);
      };
    },
    async info() {
      return { socket: "isolated test bridge", log: "test harness artifacts" };
    },
  };
}
export const createTransport = createWebTransport;
