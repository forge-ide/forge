import { type Component, type JSX, splitProps } from 'solid-js';
import type { ButtonSize } from './Button';

export type IconButtonVariant = 'ghost' | 'danger';

export interface IconButtonProps
  extends Omit<
    JSX.ButtonHTMLAttributes<HTMLButtonElement>,
    'aria-label' | 'children' | 'type' | 'title'
  > {
  /** The glyph (SVG, text, etc.) rendered inside the square trigger. */
  icon: JSX.Element;
  /**
   * Required accessible name. Forwarded as `aria-label` and as the default
   * `title` (override via the explicit `title` prop when the tooltip needs
   * a richer string, e.g. label + keyboard shortcut).
   */
  label: string;
  /**
   * Optional tooltip override. When omitted, falls back to `label` so screen
   * readers and tooltip surfaces stay in sync by default.
   */
  title?: string;
  /** Visual treatment. Defaults to `'ghost'`. */
  variant?: IconButtonVariant;
  /** Sizing token. Defaults to `'md'`. */
  size?: ButtonSize;
  /** Toggle state. When supplied, renders as `aria-pressed`. */
  pressed?: boolean;
  /** Forwarded native `type`. Defaults to `'button'`. */
  type?: 'button' | 'submit' | 'reset';
}

/**
 * Forge IconButton primitive (F-450). Square icon-only trigger with a
 * required accessible `label` — that requirement is the primary lint payoff
 * the design migration captures, so an icon-only control without a name is
 * a compile error rather than a runtime audit miss.
 */
export const IconButton: Component<IconButtonProps> = (props) => {
  const [own, rest] = splitProps(props, [
    'icon',
    'label',
    'title',
    'variant',
    'size',
    'pressed',
    'class',
    'type',
  ]);

  const variant = (): IconButtonVariant => own.variant ?? 'ghost';
  const size = (): ButtonSize => own.size ?? 'md';
  const className = (): string => {
    const parts = [
      'forge-icon-button',
      `forge-icon-button--${variant()}`,
      `forge-icon-button--${size()}`,
    ];
    if (own.class) parts.push(own.class);
    return parts.join(' ');
  };

  return (
    <button
      type={own.type ?? 'button'}
      class={className()}
      aria-label={own.label}
      title={own.title ?? own.label}
      aria-pressed={own.pressed === undefined ? undefined : own.pressed}
      {...rest}
    >
      <span class="forge-icon-button__icon" aria-hidden="true">
        {own.icon}
      </span>
    </button>
  );
};
