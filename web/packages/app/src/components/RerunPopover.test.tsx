import { describe, expect, it, vi } from 'vitest';
import { render, fireEvent } from '@solidjs/testing-library';
import { RerunPopover } from './RerunPopover';

/**
 * F-600 — RerunPopover unit tests.
 *
 * Covers:
 *   - All three variants render as buttons (Replace, Branch, Fresh).
 *   - Each click forwards the matching `RerunVariant` to `onPick`.
 *   - Each button carries a tooltip explaining its difference from the
 *     others (DoD: "tooltip explaining the difference from Replace and
 *     Branch").
 *   - Esc / outside-click triggers `onDismiss` (menu-mode focus trap).
 */
describe('RerunPopover (F-600)', () => {
  it('renders Replace, Branch, and Fresh buttons', () => {
    const { getByTestId } = render(() => (
      <RerunPopover onPick={() => undefined} onDismiss={() => undefined} />
    ));
    expect(getByTestId('rerun-popover-replace')).toBeInTheDocument();
    expect(getByTestId('rerun-popover-branch')).toBeInTheDocument();
    expect(getByTestId('rerun-popover-fresh')).toBeInTheDocument();
  });

  it('Replace button forwards "Replace" to onPick', () => {
    const onPick = vi.fn();
    const { getByTestId } = render(() => (
      <RerunPopover onPick={onPick} onDismiss={() => undefined} />
    ));
    fireEvent.click(getByTestId('rerun-popover-replace'));
    expect(onPick).toHaveBeenCalledWith('Replace');
  });

  it('Branch button forwards "Branch" to onPick', () => {
    const onPick = vi.fn();
    const { getByTestId } = render(() => (
      <RerunPopover onPick={onPick} onDismiss={() => undefined} />
    ));
    fireEvent.click(getByTestId('rerun-popover-branch'));
    expect(onPick).toHaveBeenCalledWith('Branch');
  });

  it('Fresh button forwards "Fresh" to onPick', () => {
    const onPick = vi.fn();
    const { getByTestId } = render(() => (
      <RerunPopover onPick={onPick} onDismiss={() => undefined} />
    ));
    fireEvent.click(getByTestId('rerun-popover-fresh'));
    expect(onPick).toHaveBeenCalledWith('Fresh');
  });

  // DoD: "tooltip explaining the difference from Replace and Branch"
  // The tooltip must mention enough of the contrast that a user who hovers
  // a single button understands what makes Fresh distinct. We assert the
  // verbatim words from the spec ("new root", "Replace", "Branch").
  it('Fresh button tooltip explains the difference from Replace and Branch', () => {
    const { getByTestId } = render(() => (
      <RerunPopover onPick={() => undefined} onDismiss={() => undefined} />
    ));
    const fresh = getByTestId('rerun-popover-fresh');
    const tooltip = fresh.getAttribute('title') ?? '';
    expect(tooltip).toContain('Fresh');
    expect(tooltip).toContain('Replace');
    expect(tooltip).toContain('Branch');
    expect(tooltip).toContain('new root');
  });

  it('Replace and Branch buttons each carry an explanatory tooltip', () => {
    const { getByTestId } = render(() => (
      <RerunPopover onPick={() => undefined} onDismiss={() => undefined} />
    ));
    expect(getByTestId('rerun-popover-replace').getAttribute('title')).toMatch(
      /truncate|in place|regenerate/i,
    );
    expect(getByTestId('rerun-popover-branch').getAttribute('title')).toMatch(
      /keep the original|alongside|variant/i,
    );
  });

  it('Escape key invokes onDismiss', () => {
    const onDismiss = vi.fn();
    render(() => (
      <RerunPopover onPick={() => undefined} onDismiss={onDismiss} />
    ));
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(onDismiss).toHaveBeenCalled();
  });

  it('outside-click invokes onDismiss', () => {
    const onDismiss = vi.fn();
    render(() => (
      <RerunPopover onPick={() => undefined} onDismiss={onDismiss} />
    ));
    fireEvent.mouseDown(document.body);
    expect(onDismiss).toHaveBeenCalled();
  });

  it('uses role="menu" with three menuitem children', () => {
    const { getByRole, getAllByRole } = render(() => (
      <RerunPopover onPick={() => undefined} onDismiss={() => undefined} />
    ));
    expect(getByRole('menu')).toBeInTheDocument();
    expect(getAllByRole('menuitem')).toHaveLength(3);
  });
});
