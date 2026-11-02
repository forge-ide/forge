import { For, Show, type JSX } from "solid-js";
import { createVirtualizer } from "@tanstack/solid-virtual";
/** Use a bounded viewport even for small lists; large lists never mount all rows. */
export function VirtualList<T>(props: {
  items: readonly T[];
  children: (item: T) => JSX.Element;
  label: string;
  height?: string;
  estimate?: number;
}) {
  let scroll: HTMLDivElement | undefined;
  const virtual = createVirtualizer({
    get count() {
      return props.items.length;
    },
    getScrollElement: () => scroll ?? null,
    estimateSize: () => props.estimate ?? 60,
    overscan: 4,
    initialRect: { width: 800, height: 400 },
  });
  return (
    <div
      class="virtual-list"
      aria-label={props.label}
      ref={(el) => (scroll = el)}
      style={{
        height:
          props.items.length > 100 && props.height === "auto"
            ? "min(45vh, 320px)"
            : (props.height ?? "min(55vh, 480px)"),
        overflow: "auto",
        position: "relative",
      }}
    >
      <Show
        when={props.items.length <= 100}
        fallback={
          <div
            style={{ height: `${virtual.getTotalSize()}px`, position: "relative", width: "100%" }}
          >
            <For each={virtual.getVirtualItems()}>
              {(row) => (
                <div
                  data-index={row.index}
                  ref={(el) => virtual.measureElement(el)}
                  style={{
                    position: "absolute",
                    top: 0,
                    left: 0,
                    width: "100%",
                    transform: `translateY(${row.start}px)`,
                  }}
                >
                  <Show when={props.items[row.index]}>{(item) => props.children(item())}</Show>
                </div>
              )}
            </For>
          </div>
        }
      >
        <For each={props.items}>{(item) => props.children(item)}</For>
      </Show>
    </div>
  );
}
