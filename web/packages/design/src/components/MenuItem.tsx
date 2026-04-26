import { type Component, type JSX, splitProps, Show } from 'solid-js';

export type MenuItemVariant = 'default' | 'danger';

export interface MenuItemProps
  extends Omit<JSX.ButtonHTMLAttributes<HTMLButtonElement>, 'type' | 'role'> {
  /** Optional left-rail hint (e.g. file path scope). */
  leadingText?: string;
  /** Right-aligned keyboard shortcut hint (e.g. `A`, `T`). */
  kbd?: string;
  /** Visual treatment. Defaults to `'default'`. */
  variant?: MenuItemVariant;
  /** Forwarded native `type`. Defaults to `'button'`. */
  type?: 'button' | 'submit' | 'reset';
}

/**
 * Forge MenuItem primitive (F-450). Row inside a `role="menu"` container —
 * the parent owns the menu container, the primitive owns the row's
 * `role="menuitem"` + label/leading-text/kbd layout. Disabled rows render
 * `aria-disabled` so a focused-but-disabled row keeps its keyboard hit-box.
 */
export const MenuItem: Component<MenuItemProps> = (props) => {
  const [own, rest] = splitProps(props, [
    'leadingText',
    'kbd',
    'variant',
    'class',
    'children',
    'type',
    'disabled',
  ]);
  const variant = (): MenuItemVariant => own.variant ?? 'default';
  const className = (): string => {
    const parts = ['forge-menu-item', `forge-menu-item--${variant()}`];
    if (own.class) parts.push(own.class);
    return parts.join(' ');
  };
  return (
    <button
      type={own.type ?? 'button'}
      class={className()}
      role="menuitem"
      disabled={own.disabled}
      aria-disabled={own.disabled === true ? 'true' : undefined}
      {...rest}
    >
      <Show when={own.leadingText}>
        <span class="forge-menu-item__leading">{own.leadingText}</span>
      </Show>
      <span class="forge-menu-item__label">{own.children}</span>
      <Show when={own.kbd}>
        <kbd class="forge-menu-item__kbd">{own.kbd}</kbd>
      </Show>
    </button>
  );
};
