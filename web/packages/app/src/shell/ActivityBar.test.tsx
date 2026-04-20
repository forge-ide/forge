// F-126: ActivityBar render + click behavior.
//
// The bar is purely presentational — it emits `onSelect` for enabled items
// and respects the `active` prop for visual state. Placeholder items
// (search, git) must render but not invoke `onSelect` so the chrome is
// visually complete before F-127/F-128 wire them up.

import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render } from '@solidjs/testing-library';
import { ActivityBar } from './ActivityBar';

afterEach(() => cleanup());

describe('ActivityBar', () => {
  it('renders files, search, git buttons with accessible labels', () => {
    const { getByTestId } = render(() => (
      <ActivityBar active="files" onSelect={vi.fn()} />
    ));
    expect(getByTestId('activity-bar-files')).toBeInTheDocument();
    expect(getByTestId('activity-bar-search')).toBeInTheDocument();
    expect(getByTestId('activity-bar-git')).toBeInTheDocument();
  });

  it('marks the active item with aria-pressed=true', () => {
    const { getByTestId } = render(() => (
      <ActivityBar active="files" onSelect={vi.fn()} />
    ));
    expect(getByTestId('activity-bar-files').getAttribute('aria-pressed')).toBe('true');
    expect(getByTestId('activity-bar-search').getAttribute('aria-pressed')).toBe('false');
  });

  it('invokes onSelect when the Files button is clicked', () => {
    const onSelect = vi.fn();
    const { getByTestId } = render(() => (
      <ActivityBar active={null} onSelect={onSelect} />
    ));
    fireEvent.click(getByTestId('activity-bar-files'));
    expect(onSelect).toHaveBeenCalledWith('files');
  });

  it('disables the search and git placeholders', () => {
    const onSelect = vi.fn();
    const { getByTestId } = render(() => (
      <ActivityBar active={null} onSelect={onSelect} />
    ));
    const search = getByTestId('activity-bar-search') as HTMLButtonElement;
    const git = getByTestId('activity-bar-git') as HTMLButtonElement;
    expect(search.disabled).toBe(true);
    expect(git.disabled).toBe(true);
    // Disabled buttons should not fire click handlers even if the user
    // manages to trigger them programmatically — browsers already enforce
    // this, the assertion below is belt-and-braces.
    fireEvent.click(search);
    fireEvent.click(git);
    expect(onSelect).not.toHaveBeenCalled();
  });

  it('shows the Files shortcut in the button tooltip', () => {
    const { getByTestId } = render(() => (
      <ActivityBar active={null} onSelect={vi.fn()} />
    ));
    const title = getByTestId('activity-bar-files').getAttribute('title') ?? '';
    expect(title.toLowerCase()).toContain('shift');
    expect(title.toLowerCase()).toContain('e');
  });
});
