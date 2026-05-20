// ActivityBar render + click behavior for the workspace window.
//
// The bar is purely presentational — it emits `onSelect` for enabled
// items and respects the `active` prop for visual state. Placeholder
// items (search, plugins today) must render but not invoke `onSelect`
// so the chrome is visually complete before their backing IPCs ship.

import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render } from '@solidjs/testing-library';
import { ActivityBar } from './ActivityBar';

afterEach(() => cleanup());

const ALL_IDS = ['files', 'search', 'chat', 'agents', 'skills', 'mcp', 'plugins'] as const;

describe('ActivityBar', () => {
  it('renders all seven workspace activity buttons', () => {
    const { getByTestId } = render(() => (
      <ActivityBar active="files" onSelect={vi.fn()} />
    ));
    for (const id of ALL_IDS) {
      expect(getByTestId(`activity-bar-${id}`)).toBeInTheDocument();
    }
  });

  it('marks the active item with aria-pressed=true', () => {
    const { getByTestId } = render(() => (
      <ActivityBar active="agents" onSelect={vi.fn()} />
    ));
    expect(getByTestId('activity-bar-agents').getAttribute('aria-pressed')).toBe('true');
    expect(getByTestId('activity-bar-files').getAttribute('aria-pressed')).toBe('false');
    expect(getByTestId('activity-bar-search').getAttribute('aria-pressed')).toBe('false');
  });

  it('invokes onSelect with the matching id for each enabled button', () => {
    const onSelect = vi.fn();
    const { getByTestId } = render(() => (
      <ActivityBar active={null} onSelect={onSelect} />
    ));
    for (const id of ['files', 'chat', 'agents', 'skills', 'mcp'] as const) {
      fireEvent.click(getByTestId(`activity-bar-${id}`));
      expect(onSelect).toHaveBeenCalledWith(id);
    }
  });

  it('disables the search and plugins placeholders', () => {
    const onSelect = vi.fn();
    const { getByTestId } = render(() => (
      <ActivityBar active={null} onSelect={onSelect} />
    ));
    const search = getByTestId('activity-bar-search') as HTMLButtonElement;
    const plugins = getByTestId('activity-bar-plugins') as HTMLButtonElement;
    expect(search.disabled).toBe(true);
    expect(plugins.disabled).toBe(true);
    // Disabled buttons should not fire click handlers even if the user
    // manages to trigger them programmatically — browsers already enforce
    // this, the assertion below is belt-and-braces.
    fireEvent.click(search);
    fireEvent.click(plugins);
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
