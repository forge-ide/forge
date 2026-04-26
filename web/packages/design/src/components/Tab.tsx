import { type Component, type JSX, splitProps, Show } from 'solid-js';

export type TabsVariant = 'tab' | 'radio';

export interface TabsProps
  extends Omit<JSX.HTMLAttributes<HTMLDivElement>, 'role' | 'onSelect'> {
  /** Variant — `'tab'` renders `role="tablist"`; `'radio'` renders `role="radiogroup"`. Defaults to `'tab'`. */
  variant?: TabsVariant;
}

/**
 * Container for a row of `Tab` primitives. Apply `role="tablist"` (default)
 * or `role="radiogroup"` (`variant="radio"`); the individual `Tab` children
 * pick up the matching `role` automatically through their own `variant` prop.
 *
 * Roving-tabindex / arrow-key navigation is the parent feature's
 * responsibility — this primitive owns DOM shape and ARIA role only, mirroring
 * how the existing per-feature tablists are wired (e.g. `useRovingTabindex`
 * in `app/lib/`).
 */
export const Tabs: Component<TabsProps> = (props) => {
  const [own, rest] = splitProps(props, ['variant', 'class', 'children']);
  const variant = (): TabsVariant => own.variant ?? 'tab';
  const className = (): string => {
    const parts = ['forge-tabs', `forge-tabs--${variant()}`];
    if (own.class) parts.push(own.class);
    return parts.join(' ');
  };
  return (
    <div
      class={className()}
      role={variant() === 'radio' ? 'radiogroup' : 'tablist'}
      {...rest}
    >
      {own.children}
    </div>
  );
};

export interface TabProps
  extends Omit<JSX.ButtonHTMLAttributes<HTMLButtonElement>, 'type' | 'role' | 'aria-selected' | 'aria-checked'> {
  /** Whether this tab is the currently-selected one. */
  selected: boolean;
  /** Variant — `'tab'` for `role="tab"` + `aria-selected`, `'radio'` for `role="radio"` + `aria-checked`. Defaults to `'tab'`. */
  variant?: TabsVariant;
  /** Optional badge count rendered after the label. */
  badgeCount?: number;
  /** Forwarded native `type`. Defaults to `'button'`. */
  type?: 'button' | 'submit' | 'reset';
}

/**
 * Forge Tab primitive (F-450). Renders `role="tab"` + `aria-selected`
 * (default) or `role="radio"` + `aria-checked` when `variant="radio"`; the
 * latter covers the approval-scope pill pattern that historically abused
 * `role="radio"` on a raw `<button>`.
 */
export const Tab: Component<TabProps> = (props) => {
  const [own, rest] = splitProps(props, [
    'selected',
    'variant',
    'badgeCount',
    'class',
    'children',
    'type',
  ]);
  const variant = (): TabsVariant => own.variant ?? 'tab';
  const className = (): string => {
    const parts = ['forge-tab', `forge-tab--${variant()}`];
    if (own.selected) parts.push('forge-tab--selected');
    if (own.class) parts.push(own.class);
    return parts.join(' ');
  };
  return (
    <button
      type={own.type ?? 'button'}
      class={className()}
      role={variant() === 'radio' ? 'radio' : 'tab'}
      aria-selected={variant() === 'tab' ? own.selected : undefined}
      aria-checked={variant() === 'radio' ? own.selected : undefined}
      tabIndex={own.selected ? 0 : -1}
      {...rest}
    >
      {own.children}
      <Show when={own.badgeCount !== undefined}>
        <span class="forge-tab__badge">{own.badgeCount}</span>
      </Show>
    </button>
  );
};
