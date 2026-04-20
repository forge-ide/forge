import { describe, expect, it, vi } from 'vitest';
import { render, fireEvent } from '@solidjs/testing-library';
import { ContextChip } from './ContextChip';

// F-141: ContextChip is the pill that lands in the `ctx-chips` row when
// the user picks a ContextPicker result. It owns minimal behavior — icon +
// label + dismiss — with the per-category resolution pushed to F-142.

describe('ContextChip', () => {
  it('renders the label', () => {
    const { getByTestId } = render(() => (
      <ContextChip category="file" label="src/app.ts" onDismiss={() => {}} />
    ));
    expect(getByTestId('ctx-chip')).toHaveTextContent('src/app.ts');
  });

  it('exposes the category via data attribute for downstream styling', () => {
    const { getByTestId } = render(() => (
      <ContextChip category="terminal" label="last 20 lines" onDismiss={() => {}} />
    ));
    expect(getByTestId('ctx-chip').getAttribute('data-category')).toBe('terminal');
  });

  it('invokes onDismiss when the × button is clicked', () => {
    const onDismiss = vi.fn();
    const { getByTestId } = render(() => (
      <ContextChip category="file" label="src/app.ts" onDismiss={onDismiss} />
    ));
    fireEvent.click(getByTestId('ctx-chip-dismiss'));
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  it('renders an accessible aria-label on the dismiss button', () => {
    const { getByTestId } = render(() => (
      <ContextChip category="file" label="src/app.ts" onDismiss={() => {}} />
    ));
    expect(getByTestId('ctx-chip-dismiss').getAttribute('aria-label')).toBe(
      'Remove src/app.ts',
    );
  });
});
