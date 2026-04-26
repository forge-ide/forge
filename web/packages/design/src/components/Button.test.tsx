import { describe, expect, it, vi } from 'vitest';
import { render, fireEvent, cleanup } from '@solidjs/testing-library';
import { Button } from './Button';

describe('Button', () => {
  it('renders type="button" by default — never submits a parent form', () => {
    const { getByRole } = render(() => <Button>Send</Button>);
    expect(getByRole('button').getAttribute('type')).toBe('button');
    cleanup();
  });

  it('applies the primary class by default', () => {
    const { getByRole } = render(() => <Button>Send</Button>);
    const btn = getByRole('button');
    expect(btn.classList.contains('forge-button')).toBe(true);
    expect(btn.classList.contains('forge-button--primary')).toBe(true);
    expect(btn.classList.contains('forge-button--md')).toBe(true);
    cleanup();
  });

  it.each(['primary', 'ghost', 'danger'] as const)(
    'wires variant=%s to a class modifier',
    (variant) => {
      const { getByRole } = render(() => <Button variant={variant}>Go</Button>);
      expect(getByRole('button').classList.contains(`forge-button--${variant}`)).toBe(true);
      cleanup();
    },
  );

  it('size=sm and md select the correct class modifier', () => {
    const { getByRole, unmount } = render(() => <Button size="sm">A</Button>);
    expect(getByRole('button').classList.contains('forge-button--sm')).toBe(true);
    unmount();
    const md = render(() => <Button size="md">A</Button>);
    expect(md.getByRole('button').classList.contains('forge-button--md')).toBe(true);
    md.unmount();
  });

  it('forwards onClick when not disabled', () => {
    const onClick = vi.fn();
    const { getByRole } = render(() => <Button onClick={onClick}>Send</Button>);
    fireEvent.click(getByRole('button'));
    expect(onClick).toHaveBeenCalledTimes(1);
    cleanup();
  });

  it('disabled blocks click', () => {
    const onClick = vi.fn();
    const { getByRole } = render(() => (
      <Button onClick={onClick} disabled>
        Send
      </Button>
    ));
    fireEvent.click(getByRole('button'));
    expect(onClick).not.toHaveBeenCalled();
    expect(getByRole('button').hasAttribute('disabled')).toBe(true);
    cleanup();
  });

  it('loading implies disabled and sets aria-busy', () => {
    const onClick = vi.fn();
    const { getByRole } = render(() => (
      <Button onClick={onClick} loading>
        Send
      </Button>
    ));
    const btn = getByRole('button');
    fireEvent.click(btn);
    expect(onClick).not.toHaveBeenCalled();
    expect(btn.getAttribute('aria-busy')).toBe('true');
    cleanup();
  });

  it('renders kbd hint when supplied', () => {
    const { container } = render(() => <Button kbd="↵">Send</Button>);
    const kbd = container.querySelector('kbd.forge-button__kbd');
    expect(kbd?.textContent).toBe('↵');
    cleanup();
  });

  it('forwards data-testid through ...rest', () => {
    const { getByTestId } = render(() => (
      <Button data-testid="composer-send-btn">Send</Button>
    ));
    expect(getByTestId('composer-send-btn')).toBeInstanceOf(HTMLButtonElement);
    cleanup();
  });

  it('merges caller class with primitive class', () => {
    const { getByRole } = render(() => <Button class="composer__btn">Send</Button>);
    const btn = getByRole('button');
    expect(btn.classList.contains('forge-button')).toBe(true);
    expect(btn.classList.contains('composer__btn')).toBe(true);
    cleanup();
  });

  it('focus-visible ring is reachable through keyboard focus', () => {
    const { getByRole } = render(() => <Button>Send</Button>);
    const btn = getByRole('button') as HTMLButtonElement;
    btn.focus();
    expect(document.activeElement).toBe(btn);
    cleanup();
  });
});
