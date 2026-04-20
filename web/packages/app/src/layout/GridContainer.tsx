import { type Component, type JSX, Match, Show, Switch } from 'solid-js';
import { SplitPane } from './SplitPane';
import { DropZoneOverlay } from './DropZoneOverlay';
import type { DragState } from './useDragToDock';

/**
 * A terminal node in the layout tree. `render` produces the pane body for
 * the leaf — F-117 stays agnostic about pane type (chat, terminal, editor,
 * files, agent monitor); the caller supplies the renderer.
 */
export interface LeafNode {
  kind: 'leaf';
  id: string;
  render: () => JSX.Element;
}

/**
 * An internal node that splits its area between `a` and `b`. `ratio` is the
 * fraction of the container occupied by `a` (0..1).
 */
export interface SplitNode {
  kind: 'split';
  id: string;
  direction: 'h' | 'v';
  ratio: number;
  a: LayoutNode;
  b: LayoutNode;
}

export type LayoutNode = LeafNode | SplitNode;

export interface GridContainerProps {
  /** The root of the layout tree. Arbitrarily deep H/V combinations. */
  tree: LayoutNode;
  /**
   * Emitted when a split node's ratio changes (drag or double-click
   * reset). `id` is the split node's id; the parent owns persistence
   * and tree mutation. GridContainer keeps no state of its own.
   */
  onRatioChange: (id: string, next: number) => void;
  /**
   * Active drag-to-dock state, threaded from `useDragToDock`. When set,
   * GridContainer paints a `DropZoneOverlay` on the targeted leaf so the
   * hovered zone gets the §3.6 ember tint. Optional — omit when drag
   * isn't wired up (e.g. unit tests for ratio behavior).
   */
  dragState?: DragState | null;
}

/**
 * Recursive N-ary layout renderer over `SplitPane`. Each split node maps to
 * one `SplitPane`; each leaf renders via its callback. GridContainer is a
 * pure function of its props — the tree walks parent-side, which keeps
 * persistence (F-120) and drag-to-dock (F-118) trivial to layer on later.
 *
 * F-118 layered the optional `dragState` prop here so the drop-zone overlay
 * co-locates with the leaves. Tree mutation still happens outside — the
 * `useDragToDock` hook calls `applyDockDrop` and hands the result back to
 * the parent via `onTreeChange`. GridContainer stays stateless.
 */
export const GridContainer: Component<GridContainerProps> = (props) => {
  return (
    <GridNode
      node={props.tree}
      onRatioChange={props.onRatioChange}
      dragState={() => props.dragState ?? null}
    />
  );
};

const GridNode: Component<{
  node: LayoutNode;
  onRatioChange: (id: string, next: number) => void;
  dragState: () => DragState | null;
}> = (props) => {
  return (
    <Switch>
      <Match when={props.node.kind === 'leaf' && (props.node as LeafNode)}>
        {(leaf) => {
          const activeZone = () => {
            const s = props.dragState();
            return s !== null && s.targetId === leaf().id ? s.zone : null;
          };
          const showOverlay = () => props.dragState() !== null;
          return (
            <div
              class="grid-leaf"
              data-testid={`grid-leaf-${leaf().id}`}
              data-leaf-id={leaf().id}
              style={{ width: '100%', height: '100%', position: 'relative' }}
            >
              {leaf().render()}
              <Show when={showOverlay()}>
                <DropZoneOverlay activeZone={activeZone()} />
              </Show>
            </div>
          );
        }}
      </Match>
      <Match when={props.node.kind === 'split' && (props.node as SplitNode)}>
        {(split) => (
          <SplitPane
            direction={split().direction}
            ratio={split().ratio}
            onRatioChange={(next) => props.onRatioChange(split().id, next)}
          >
            <GridNode
              node={split().a}
              onRatioChange={props.onRatioChange}
              dragState={props.dragState}
            />
            <GridNode
              node={split().b}
              onRatioChange={props.onRatioChange}
              dragState={props.dragState}
            />
          </SplitPane>
        )}
      </Match>
    </Switch>
  );
};
