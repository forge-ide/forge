import { describe, expect, it, vi } from 'vitest';
import { render, fireEvent, cleanup } from '@solidjs/testing-library';
import { MenuItem } from './MenuItem';

describe('MenuItem', () => {
  it('renders role="menuitem"', () => {
    const { getByRole } = render(() => <MenuItem>Open</MenuItem>);
    expect(getByRole('menuitem')).toBeTruthy();
    cleanup();
  });

  it('renders type="button" by default', () => {
    const { getByRole } = render(() => <MenuItem>Open</MenuItem>);
    expect(getByRole('menuitem').getAttribute('type')).toBe('button');
    cleanup();
  });

  it('forwards onClick when not disabled', () => {
    const onClick = vi.fn();
    const { getByRole } = render(() => (
      <MenuItem onClick={onClick}>Open</MenuItem>
    ));
    fireEvent.click(getByRole('menuitem'));
    expect(onClick).toHaveBeenCalledTimes(1);
    cleanup();
  });

  it('disabled blocks click and sets aria-disabled', () => {
    const onClick = vi.fn();
    const { getByRole } = render(() => (
      <MenuItem onClick={onClick} disabled>
        Open
      </MenuItem>
    ));
    const item = getByRole('menuitem');
    fireEvent.click(item);
    expect(onClick).not.toHaveBeenCalled();
    expect(item.getAttribute('aria-disabled')).toBe('true');
    cleanup();
  });

  it('renders kbd hint when supplied', () => {
    const { container } = render(() => (
      <MenuItem kbd="A">Approve once</MenuItem>
    ));
    expect(container.querySelector('kbd.forge-menu-item__kbd')?.textContent).toBe('A');
    cleanup();
  });

  it('renders leadingText when supplied', () => {
    const { container } = render(() => (
      <MenuItem leadingText="src/foo.ts">Open file</MenuItem>
    ));
    expect(container.querySelector('.forge-menu-item__leading')?.textContent).toBe(
      'src/foo.ts',
    );
    cleanup();
  });

  it('danger variant gets the danger modifier class', () => {
    const { getByRole } = render(() => (
      <MenuItem variant="danger">Delete</MenuItem>
    ));
    expect(
      getByRole('menuitem').classList.contains('forge-menu-item--danger'),
    ).toBe(true);
    cleanup();
  });
});
