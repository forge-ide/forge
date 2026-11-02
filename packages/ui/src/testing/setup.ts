import { vi } from "vitest";
// jsdom does not perform layout; browser performance is checked by the e2e harness.
Object.defineProperty(window, "matchMedia", {
  value: () => ({ matches: false, addEventListener() {}, removeEventListener() {} }),
});
if (!globalThis.ResizeObserver)
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
Range.prototype.getBoundingClientRect = () => new DOMRect();
vi.stubGlobal("requestAnimationFrame", (fn: FrameRequestCallback) =>
  setTimeout(() => fn(performance.now()), 16),
);
vi.stubGlobal("cancelAnimationFrame", clearTimeout);
