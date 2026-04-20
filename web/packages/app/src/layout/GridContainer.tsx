import { type Component, type JSX, Match, Switch } from 'solid-js';
import { SplitPane } from './SplitPane';

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
}

/**
 * Recursive N-ary layout renderer over `SplitPane`. Each split node maps to
 * one `SplitPane`; each leaf renders via its callback. GridContainer is a
 * pure function of its props — the tree walks parent-side, which keeps
 * persistence (F-120) and drag-to-dock (F-118) trivial to layer on later.
 */
export const GridContainer: Component<GridContainerProps> = (props) => {
  return <GridNode node={props.tree} onRatioChange={props.onRatioChange} />;
};

const GridNode: Component<{
  node: LayoutNode;
  onRatioChange: (id: string, next: number) => void;
}> = (props) => {
  return (
    <Switch>
      <Match when={props.node.kind === 'leaf' && (props.node as LeafNode)}>
        {(leaf) => (
          <div
            class="grid-leaf"
            data-testid={`grid-leaf-${leaf().id}`}
            data-leaf-id={leaf().id}
            style={{ width: '100%', height: '100%' }}
          >
            {leaf().render()}
          </div>
        )}
      </Match>
      <Match when={props.node.kind === 'split' && (props.node as SplitNode)}>
        {(split) => (
          <SplitPane
            direction={split().direction}
            ratio={split().ratio}
            onRatioChange={(next) => props.onRatioChange(split().id, next)}
          >
            <GridNode node={split().a} onRatioChange={props.onRatioChange} />
            <GridNode node={split().b} onRatioChange={props.onRatioChange} />
          </SplitPane>
        )}
      </Match>
    </Switch>
  );
};
