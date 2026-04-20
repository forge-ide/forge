import type { LayoutNode, LeafNode, SplitNode } from './GridContainer';

/**
 * Which edge (or center) of a target pane is being hovered. Matches the five
 * zones from `docs/ui-specs/layout-panes.md` §3.6:
 *   - `left` / `right`: vertical split with the source on that side
 *   - `top` / `bottom`: horizontal split with the source on that side
 *   - `center`: replace the target leaf with the source leaf (see note below)
 *
 * Note: §3.6 describes the center drop as "move as a tab into the target pane
 * (if both panes are the same type)". F-117 leaves are intentionally type-
 * agnostic — there's no tab model yet (F-122 covers tabs). F-118 lands
 * `center` as a **replace** so the interaction is functional end-to-end; the
 * behavior upgrades to tab-merge once F-122 lands. This is called out in the
 * F-118 PR description.
 */
export type DropZone = 'top' | 'bottom' | 'left' | 'right' | 'center';

/**
 * Fraction of the pane's width/height reserved for edge zones on each side.
 * The inner square (the remaining `1 - 2 * EDGE_ZONE_FRACTION` on each axis)
 * is the center zone. 0.25 keeps each edge strip at 25% of the pane, which
 * gives a generous target without shrinking the center too aggressively.
 */
export const EDGE_ZONE_FRACTION = 0.25;

/**
 * Classify a pointer position inside a pane's bounding rect into a drop zone.
 * Returns `null` when the pointer is outside the rect. The priority order —
 * `top` / `bottom` when the pointer is within the vertical edge strip wins
 * over `left` / `right` in the corners — matches the standard VS Code-like
 * behavior where the closer edge wins.
 */
export function zoneForPoint(
  clientX: number,
  clientY: number,
  rect: { left: number; top: number; right: number; bottom: number; width: number; height: number },
): DropZone | null {
  if (clientX < rect.left || clientX > rect.right) return null;
  if (clientY < rect.top || clientY > rect.bottom) return null;
  const fx = (clientX - rect.left) / rect.width;
  const fy = (clientY - rect.top) / rect.height;
  // Prefer the nearest edge — whichever of (fx, 1-fx, fy, 1-fy) is smallest
  // and below the threshold wins. Ties break left → right → top → bottom.
  const candidates: Array<{ zone: DropZone; d: number }> = [
    { zone: 'left', d: fx },
    { zone: 'right', d: 1 - fx },
    { zone: 'top', d: fy },
    { zone: 'bottom', d: 1 - fy },
  ];
  let best: { zone: DropZone; d: number } | null = null;
  for (const c of candidates) {
    if (c.d <= EDGE_ZONE_FRACTION && (best === null || c.d < best.d)) {
      best = c;
    }
  }
  return best !== null ? best.zone : 'center';
}

/** Find a leaf with the given id anywhere in the tree. */
function findLeaf(tree: LayoutNode, id: string): LeafNode | null {
  if (tree.kind === 'leaf') return tree.id === id ? tree : null;
  return findLeaf(tree.a, id) ?? findLeaf(tree.b, id);
}

/** True if `ancestorId` names a node whose subtree contains `id`. */
function containsId(tree: LayoutNode, id: string): boolean {
  if (tree.kind === 'leaf') return tree.id === id;
  return tree.id === id || containsId(tree.a, id) || containsId(tree.b, id);
}

/**
 * Remove the leaf with `id` from `tree` by promoting its split sibling.
 * Returns `null` if `tree` itself is the leaf (caller handles root removal)
 * or a new tree with the leaf excised. Split nodes are structurally rebuilt
 * only along the path to the removed leaf — other subtrees are shared.
 */
function removeLeaf(tree: LayoutNode, id: string): LayoutNode | null {
  if (tree.kind === 'leaf') return tree.id === id ? null : tree;
  // If one immediate child is the leaf being removed, collapse to the other.
  if (tree.a.kind === 'leaf' && tree.a.id === id) return tree.b;
  if (tree.b.kind === 'leaf' && tree.b.id === id) return tree.a;
  // Recurse. Exactly one side contains the leaf (ids are unique); if neither
  // does we return the tree unchanged.
  if (containsId(tree.a, id)) {
    const nextA = removeLeaf(tree.a, id);
    return nextA === null ? tree.b : { ...tree, a: nextA };
  }
  if (containsId(tree.b, id)) {
    const nextB = removeLeaf(tree.b, id);
    return nextB === null ? tree.a : { ...tree, b: nextB };
  }
  return tree;
}

/** Build a deterministic id for a split synthesized by a dock drop. */
function makeSplitId(sourceId: string, targetId: string, zone: DropZone): string {
  return `dock-${zone}-${sourceId}-${targetId}`;
}

/**
 * Replace the target leaf with a new subtree produced by `build(targetLeaf)`.
 * Returns the mutated tree. The caller guarantees the target exists.
 */
function replaceLeaf(
  tree: LayoutNode,
  targetId: string,
  build: (leaf: LeafNode) => LayoutNode,
): LayoutNode {
  if (tree.kind === 'leaf') return tree.id === targetId ? build(tree) : tree;
  if (containsId(tree.a, targetId)) {
    return { ...tree, a: replaceLeaf(tree.a, targetId, build) };
  }
  if (containsId(tree.b, targetId)) {
    return { ...tree, b: replaceLeaf(tree.b, targetId, build) };
  }
  return tree;
}

/**
 * Apply a dock drop to the layout tree. Pure — returns a new tree.
 *
 * For edge zones, the target leaf becomes a SplitPane containing the source
 * and the target in the order dictated by §3.6:
 *   - `right`: v-split, target=a, source=b
 *   - `left`:  v-split, source=a, target=b
 *   - `bottom`: h-split, target=a, source=b
 *   - `top`:    h-split, source=a, target=b
 *
 * For `center`, the source leaf replaces the target. (See `DropZone` docs for
 * why this is a replace, not a tab-merge.)
 *
 * The source leaf is removed from its old location — its sibling is promoted.
 * Several malformed drops are no-ops (returning the tree unchanged):
 *   - `sourceId === targetId` (nothing to move)
 *   - Either id missing from the tree
 *   - Source is an ancestor of target (would orphan the moved subtree — can't
 *     happen today because sources are always leaves, but the guard is cheap)
 */
export function applyDockDrop(
  tree: LayoutNode,
  sourceId: string,
  targetId: string,
  zone: DropZone,
): LayoutNode {
  if (sourceId === targetId) return tree;
  const source = findLeaf(tree, sourceId);
  const target = findLeaf(tree, targetId);
  if (source === null || target === null) return tree;
  // Removing the source would leave an empty tree (source is the only leaf).
  if (tree.kind === 'leaf' && tree.id === sourceId) return tree;

  // Step 1: excise the source from its current location.
  const withoutSource = removeLeaf(tree, sourceId);
  if (withoutSource === null) return tree; // defensive; covered by root guard.

  // Step 2: weave the source back in at the target site per zone.
  if (zone === 'center') {
    return replaceLeaf(withoutSource, targetId, () => source);
  }
  const direction: 'h' | 'v' = zone === 'left' || zone === 'right' ? 'v' : 'h';
  const sourceFirst = zone === 'left' || zone === 'top';
  return replaceLeaf(withoutSource, targetId, (leaf): SplitNode => ({
    kind: 'split',
    id: makeSplitId(sourceId, targetId, zone),
    direction,
    ratio: 0.5,
    a: sourceFirst ? source : leaf,
    b: sourceFirst ? leaf : source,
  }));
}
