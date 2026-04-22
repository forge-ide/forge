import { describe, expect, it, vi } from 'vitest';
import { render, fireEvent } from '@solidjs/testing-library';
import { BranchMetadataPopover, type VariantRow } from './BranchMetadataPopover';

const rows: VariantRow[] = [
  {
    index: 0,
    message_id: 'root-1',
    model: 'sonnet-4.5',
    at: '2026-04-20T14:22:11Z',
    preview: "I'll read the current…",
  },
  {
    index: 1,
    message_id: 'var-1',
    model: 'sonnet-4.5',
    at: '2026-04-20T14:24:35Z',
    preview: 'Understood. Re-drafting…',
  },
  {
    index: 2,
    message_id: 'var-2',
    model: 'opus-4.1',
    at: '2026-04-20T14:26:02Z',
    preview: 'Let me take a different…',
  },
];

describe('BranchMetadataPopover', () => {
  it('renders a header showing the live variant count', () => {
    const { getByTestId } = render(() => (
      <BranchMetadataPopover
        variants={rows}
        activeVariantId="var-1"
        onSelect={() => {}}
        onDelete={() => {}}
        onExportAll={() => {}}
        onDismiss={() => {}}
      />
    ));
    expect(getByTestId('branch-metadata-popover').textContent).toContain('3 variants');
  });

  it('renders one row per variant with the model + preview', () => {
    const { getByTestId } = render(() => (
      <BranchMetadataPopover
        variants={rows}
        activeVariantId="var-1"
        onSelect={() => {}}
        onDelete={() => {}}
        onExportAll={() => {}}
        onDismiss={() => {}}
      />
    ));
    const row1 = getByTestId('branch-popover-row-1');
    expect(row1.textContent).toContain('variant 1');
    expect(row1.textContent).toContain('sonnet-4.5');
    expect(row1.textContent).toContain('Re-drafting');
  });

  it('marks the active row with aria-current', () => {
    const { getByTestId } = render(() => (
      <BranchMetadataPopover
        variants={rows}
        activeVariantId="var-1"
        onSelect={() => {}}
        onDelete={() => {}}
        onExportAll={() => {}}
        onDismiss={() => {}}
      />
    ));
    const activeBtn = getByTestId('branch-popover-select-1');
    expect(activeBtn.getAttribute('aria-current')).toBe('true');
  });

  it('clicking a non-active row fires onSelect with the row message_id', () => {
    const onSelect = vi.fn();
    const { getByTestId } = render(() => (
      <BranchMetadataPopover
        variants={rows}
        activeVariantId="var-1"
        onSelect={onSelect}
        onDelete={() => {}}
        onExportAll={() => {}}
        onDismiss={() => {}}
      />
    ));
    fireEvent.click(getByTestId('branch-popover-select-2'));
    expect(onSelect).toHaveBeenCalledWith('var-2');
  });

  it('delete button fires onDelete with the variant index', () => {
    const onDelete = vi.fn();
    const { getByTestId } = render(() => (
      <BranchMetadataPopover
        variants={rows}
        activeVariantId="var-1"
        onSelect={() => {}}
        onDelete={onDelete}
        onExportAll={() => {}}
        onDismiss={() => {}}
      />
    ));
    fireEvent.click(getByTestId('branch-popover-delete-1'));
    expect(onDelete).toHaveBeenCalledWith(1);
  });

  it('disables Delete on the root row when siblings remain', () => {
    const { getByTestId } = render(() => (
      <BranchMetadataPopover
        variants={rows}
        activeVariantId="root-1"
        onSelect={() => {}}
        onDelete={() => {}}
        onExportAll={() => {}}
        onDismiss={() => {}}
      />
    ));
    const rootDelete = getByTestId('branch-popover-delete-0') as HTMLButtonElement;
    expect(rootDelete.disabled).toBe(true);
  });

  it('Export button fires onExportAll', () => {
    const onExportAll = vi.fn();
    const { getByTestId } = render(() => (
      <BranchMetadataPopover
        variants={rows}
        activeVariantId="var-1"
        onSelect={() => {}}
        onDelete={() => {}}
        onExportAll={onExportAll}
        onDismiss={() => {}}
      />
    ));
    fireEvent.click(getByTestId('branch-popover-export'));
    expect(onExportAll).toHaveBeenCalledTimes(1);
  });

  it('Escape fires onDismiss', () => {
    const onDismiss = vi.fn();
    const { getByTestId } = render(() => (
      <BranchMetadataPopover
        variants={rows}
        activeVariantId="var-1"
        onSelect={() => {}}
        onDelete={() => {}}
        onExportAll={() => {}}
        onDismiss={onDismiss}
      />
    ));
    fireEvent.keyDown(getByTestId('branch-metadata-popover'), { key: 'Escape' });
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  // F-402: downgrade from role=dialog to role=menu — this surface is not
  // modal, it is a non-modal list with its own dismissal affordances.
  it('uses role="menu" (not role="dialog") — content is not truly modal', () => {
    const { getByTestId } = render(() => (
      <BranchMetadataPopover
        variants={rows}
        activeVariantId="var-1"
        onSelect={() => {}}
        onDelete={() => {}}
        onExportAll={() => {}}
        onDismiss={() => {}}
      />
    ));
    const popover = getByTestId('branch-metadata-popover');
    expect(popover.getAttribute('role')).toBe('menu');
  });

  it('outside-click fires onDismiss', () => {
    const onDismiss = vi.fn();
    render(() => (
      <BranchMetadataPopover
        variants={rows}
        activeVariantId="var-1"
        onSelect={() => {}}
        onDelete={() => {}}
        onExportAll={() => {}}
        onDismiss={onDismiss}
      />
    ));
    // Click somewhere that is not inside the popover — the document body.
    fireEvent.mouseDown(document.body);
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  it('inside-click does NOT fire onDismiss', () => {
    const onDismiss = vi.fn();
    const { getByTestId } = render(() => (
      <BranchMetadataPopover
        variants={rows}
        activeVariantId="var-1"
        onSelect={() => {}}
        onDelete={() => {}}
        onExportAll={() => {}}
        onDismiss={onDismiss}
      />
    ));
    fireEvent.mouseDown(getByTestId('branch-popover-row-1'));
    expect(onDismiss).not.toHaveBeenCalled();
  });
});
