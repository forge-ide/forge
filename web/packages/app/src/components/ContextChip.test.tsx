import { describe, expect, it, vi } from 'vitest';
import { render, fireEvent, waitFor } from '@solidjs/testing-library';
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

  // -------------------------------------------------------------------------
  // F-142 — lazy file preview popover
  // -------------------------------------------------------------------------

  it('shows a preview popover on hover for a file chip with loadPreview', async () => {
    const loadPreview = vi.fn().mockResolvedValue('console.log("hi");');
    const { getByTestId, queryByTestId } = render(() => (
      <ContextChip
        category="file"
        label="app.ts"
        value="/ws/app.ts"
        loadPreview={loadPreview}
        onDismiss={() => {}}
      />
    ));
    expect(queryByTestId('ctx-chip-preview')).toBeNull();
    fireEvent.mouseEnter(getByTestId('ctx-chip'));
    expect(loadPreview).toHaveBeenCalledWith('/ws/app.ts');
    await waitFor(() =>
      expect(getByTestId('ctx-chip-preview')).toHaveTextContent(
        'console.log("hi");',
      ),
    );
  });

  it('does not load a preview for a non-file chip', () => {
    const loadPreview = vi.fn().mockResolvedValue('ignored');
    const { getByTestId, queryByTestId } = render(() => (
      <ContextChip
        category="terminal"
        label="last 20 lines"
        loadPreview={loadPreview}
        onDismiss={() => {}}
      />
    ));
    fireEvent.mouseEnter(getByTestId('ctx-chip'));
    expect(loadPreview).not.toHaveBeenCalled();
    expect(queryByTestId('ctx-chip-preview')).toBeNull();
  });

  it('hides the preview again on mouse leave', async () => {
    const loadPreview = vi.fn().mockResolvedValue('body');
    const { getByTestId, queryByTestId } = render(() => (
      <ContextChip
        category="file"
        label="app.ts"
        value="/ws/app.ts"
        loadPreview={loadPreview}
        onDismiss={() => {}}
      />
    ));
    fireEvent.mouseEnter(getByTestId('ctx-chip'));
    await waitFor(() => expect(getByTestId('ctx-chip-preview')).toBeDefined());
    fireEvent.mouseLeave(getByTestId('ctx-chip'));
    expect(queryByTestId('ctx-chip-preview')).toBeNull();
  });

  // -------------------------------------------------------------------------
  // F-399: error state, retry, role fix
  // -------------------------------------------------------------------------

  it('shows an error message in the popover when loadPreview rejects', async () => {
    const loadPreview = vi.fn().mockRejectedValue(new Error('not found'));
    const { getByTestId } = render(() => (
      <ContextChip
        category="file"
        label="app.ts"
        value="/ws/app.ts"
        loadPreview={loadPreview}
        onDismiss={() => {}}
      />
    ));
    fireEvent.mouseEnter(getByTestId('ctx-chip'));
    await waitFor(() =>
      expect(getByTestId('ctx-chip-preview')).toHaveTextContent('not found'),
    );
  });

  it('shows a RETRY button when the preview errors', async () => {
    const loadPreview = vi.fn().mockRejectedValue(new Error('oops'));
    const { getByTestId } = render(() => (
      <ContextChip
        category="file"
        label="app.ts"
        value="/ws/app.ts"
        loadPreview={loadPreview}
        onDismiss={() => {}}
      />
    ));
    fireEvent.mouseEnter(getByTestId('ctx-chip'));
    await waitFor(() => expect(getByTestId('ctx-chip-retry')).toBeInTheDocument());
  });

  it('retry button triggers a fresh load and shows the preview on success', async () => {
    let attempt = 0;
    const loadPreview = vi.fn().mockImplementation(() => {
      attempt++;
      if (attempt === 1) return Promise.reject(new Error('transient'));
      return Promise.resolve('recovered content');
    });
    const { getByTestId } = render(() => (
      <ContextChip
        category="file"
        label="app.ts"
        value="/ws/app.ts"
        loadPreview={loadPreview}
        onDismiss={() => {}}
      />
    ));
    fireEvent.mouseEnter(getByTestId('ctx-chip'));
    await waitFor(() => expect(getByTestId('ctx-chip-retry')).toBeInTheDocument());

    fireEvent.mouseDown(getByTestId('ctx-chip-retry'));
    await waitFor(() =>
      expect(getByTestId('ctx-chip-preview')).toHaveTextContent('recovered content'),
    );
  });

  it('clears the error state at the start of a new hover after a previous error', async () => {
    let attempt = 0;
    const loadPreview = vi.fn().mockImplementation(() => {
      attempt++;
      if (attempt === 1) return Promise.reject(new Error('first error'));
      return Promise.resolve('ok');
    });
    const { getByTestId, queryByTestId } = render(() => (
      <ContextChip
        category="file"
        label="app.ts"
        value="/ws/app.ts"
        loadPreview={loadPreview}
        onDismiss={() => {}}
      />
    ));
    // First hover — error.
    fireEvent.mouseEnter(getByTestId('ctx-chip'));
    await waitFor(() => expect(getByTestId('ctx-chip-preview')).toHaveTextContent('first error'));

    // Leave and re-enter — should clear error and attempt a fresh load.
    fireEvent.mouseLeave(getByTestId('ctx-chip'));
    fireEvent.mouseEnter(getByTestId('ctx-chip'));
    await waitFor(() =>
      expect(getByTestId('ctx-chip-preview')).toHaveTextContent('ok'),
    );
    expect(queryByTestId('ctx-chip-retry')).not.toBeInTheDocument();
  });

  it('chip root has role="group" (not a button or tooltip)', () => {
    const { getByTestId } = render(() => (
      <ContextChip category="file" label="main.ts" onDismiss={() => {}} />
    ));
    expect(getByTestId('ctx-chip')).toHaveAttribute('role', 'group');
  });
});
