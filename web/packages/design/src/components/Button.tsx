import { type Component, type JSX, splitProps, Show } from 'solid-js';

/** Visual variant. */
export type ButtonVariant = 'primary' | 'ghost' | 'danger';

/** Sizing token. */
export type ButtonSize = 'sm' | 'md';

export interface ButtonProps
  extends Omit<JSX.ButtonHTMLAttributes<HTMLButtonElement>, 'type'> {
  /** Visual treatment. Defaults to `'primary'`. */
  variant?: ButtonVariant;
  /** Padding/typography size. Defaults to `'md'`. */
  size?: ButtonSize;
  /** Icon rendered before the label. */
  leadingIcon?: JSX.Element;
  /** Icon rendered after the label. */
  trailingIcon?: JSX.Element;
  /** Loading state — also disables the click and sets `aria-busy`. */
  loading?: boolean;
  /** Right-aligned keyboard-shortcut hint (e.g. `↵`, `A`). */
  kbd?: string;
  /**
   * Forwarded to the native `type` attribute. Defaults to `'button'` so a
   * stray button inside a `<form>` never accidentally submits — this is the
   * single most common foot-gun the primitive closes.
   */
  type?: 'button' | 'submit' | 'reset';
}

/**
 * Forge Button primitive (F-450). Three variants — `primary`, `ghost`,
 * `danger` — render with the four-part state machine documented in
 * `docs/design/component-principles.md §Buttons`. Children supply the verb-
 * noun display-caps label; pass `kbd` for a trailing keyboard hint.
 */
export const Button: Component<ButtonProps> = (props) => {
  const [own, rest] = splitProps(props, [
    'variant',
    'size',
    'leadingIcon',
    'trailingIcon',
    'loading',
    'kbd',
    'class',
    'children',
    'disabled',
    'type',
    'aria-busy',
  ]);

  const variant = (): ButtonVariant => own.variant ?? 'primary';
  const size = (): ButtonSize => own.size ?? 'md';
  const isDisabled = (): boolean => own.disabled === true || own.loading === true;
  const className = (): string => {
    const parts = ['forge-button', `forge-button--${variant()}`, `forge-button--${size()}`];
    if (own.class) parts.push(own.class);
    return parts.join(' ');
  };

  return (
    <button
      type={own.type ?? 'button'}
      class={className()}
      disabled={isDisabled()}
      aria-busy={own.loading === true ? 'true' : own['aria-busy']}
      {...rest}
    >
      <Show when={own.leadingIcon}>
        <span class="forge-button__leading" aria-hidden="true">
          {own.leadingIcon}
        </span>
      </Show>
      {own.children}
      <Show when={own.trailingIcon}>
        <span class="forge-button__trailing" aria-hidden="true">
          {own.trailingIcon}
        </span>
      </Show>
      <Show when={own.kbd}>
        <kbd class="forge-button__kbd">{own.kbd}</kbd>
      </Show>
    </button>
  );
};
