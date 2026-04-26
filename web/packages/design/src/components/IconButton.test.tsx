import { describe, expect, it, vi } from 'vitest';
import { render, fireEvent, cleanup } from '@solidjs/testing-library';
import { IconButton } from './IconButton';

describe('IconButton', () => {
  it('renders type="button" by default', () => {
    const { getByRole } = render(() => <IconButton icon="×" label="Close" />);
    expect(getByRole('button').getAttribute('type')).toBe('button');
    cleanup();
  });

  it('label drives both aria-label and title', () => {
    const { getByRole } = render(() => <IconButton icon="×" label="Close pane" />);
    const btn = getByRole('button');
    expect(btn.getAttribute('aria-label')).toBe('Close pane');
    expect(btn.getAttribute('title')).toBe('Close pane');
    cleanup();
  });

  it('applies the ghost variant class by default', () => {
    const { getByRole } = render(() => <IconButton icon="×" label="X" />);
    const btn = getByRole('button');
    expect(btn.classList.contains('forge-icon-button')).toBe(true);
    expect(btn.classList.contains('forge-icon-button--ghost')).toBe(true);
    expect(btn.classList.contains('forge-icon-button--md')).toBe(true);
    cleanup();
  });

  it('pressed wires aria-pressed', () => {
    const { getByRole, unmount } = render(() => (
      <IconButton icon="◐" label="Toggle" pressed={true} />
    ));
    expect(getByRole('button').getAttribute('aria-pressed')).toBe('true');
    unmount();
    const off = render(() => <IconButton icon="◐" label="Toggle" pressed={false} />);
    expect(off.getByRole('button').getAttribute('aria-pressed')).toBe('false');
    off.unmount();
  });

  it('omits aria-pressed when pressed is undefined', () => {
    const { getByRole } = render(() => <IconButton icon="×" label="X" />);
    expect(getByRole('button').hasAttribute('aria-pressed')).toBe(false);
    cleanup();
  });

  it('forwards onClick', () => {
    const onClick = vi.fn();
    const { getByRole } = render(() => (
      <IconButton icon="×" label="Close" onClick={onClick} />
    ));
    fireEvent.click(getByRole('button'));
    expect(onClick).toHaveBeenCalledTimes(1);
    cleanup();
  });

  it('disabled blocks click', () => {
    const onClick = vi.fn();
    const { getByRole } = render(() => (
      <IconButton icon="×" label="Close" onClick={onClick} disabled />
    ));
    fireEvent.click(getByRole('button'));
    expect(onClick).not.toHaveBeenCalled();
    cleanup();
  });
});
