import { describe, expect, it, vi, beforeEach } from 'vitest';
import { render, fireEvent } from '@solidjs/testing-library';
import { ApprovalPrompt } from './ApprovalPrompt';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const FS_EDIT_ARGS = JSON.stringify({ path: '/src/foo.ts', patch: '...' });
const SHELL_ARGS = JSON.stringify({ command: '/bin/sh', args: ['-c', 'echo hi'] });
const PREVIEW = { description: 'Edit file /src/foo.ts: 3 hunks, +47 -21' };

function makeContainer(): HTMLDivElement {
  const el = document.createElement('div');
  el.tabIndex = 0;
  document.body.appendChild(el);
  return el;
}

function renderPrompt(
  overrides: Partial<{
    toolName: string;
    argsJson: string;
    onApprove: ReturnType<typeof vi.fn>;
    onReject: ReturnType<typeof vi.fn>;
  }> = {},
) {
  const container = makeContainer();
  const onApprove = overrides.onApprove ?? vi.fn();
  const onReject = overrides.onReject ?? vi.fn();

  const result = render(
    () => (
      <ApprovalPrompt
        toolCallId="tc-test"
        toolName={overrides.toolName ?? 'fs.edit'}
        argsJson={overrides.argsJson ?? FS_EDIT_ARGS}
        preview={PREVIEW}
        containerRef={container}
        onApprove={onApprove}
        onReject={onReject}
      />
    ),
    { container },
  );

  return { ...result, container, onApprove, onReject };
}

beforeEach(() => {
  // Remove any appended containers from the previous test
  while (document.body.firstChild) {
    document.body.removeChild(document.body.firstChild);
  }
});

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

describe('ApprovalPrompt rendering', () => {
  it('renders the approval prompt container', () => {
    const { getByTestId } = renderPrompt();
    expect(getByTestId('approval-prompt')).toBeInTheDocument();
  });

  it('renders the preview text', () => {
    const { getByTestId } = renderPrompt();
    expect(getByTestId('approval-preview')).toHaveTextContent('Edit file /src/foo.ts');
  });

  it('renders reject and approve buttons', () => {
    const { getByTestId } = renderPrompt();
    expect(getByTestId('reject-btn')).toBeInTheDocument();
    expect(getByTestId('approve-once-btn')).toBeInTheDocument();
    expect(getByTestId('approve-dropdown-btn')).toBeInTheDocument();
  });

  it('shows file-scope buttons for fs.edit with a path', () => {
    const { getByTestId } = renderPrompt({ toolName: 'fs.edit', argsJson: FS_EDIT_ARGS });
    fireEvent.click(getByTestId('approve-dropdown-btn'));
    expect(getByTestId('scope-file-btn')).toBeInTheDocument();
    expect(getByTestId('scope-pattern-btn')).toBeInTheDocument();
  });

  it('shows file-scope buttons for fs.write with a path', () => {
    const { getByTestId } = renderPrompt({
      toolName: 'fs.write',
      argsJson: JSON.stringify({ path: '/src/foo.ts', content: 'hello' }),
    });
    fireEvent.click(getByTestId('approve-dropdown-btn'));
    expect(getByTestId('scope-file-btn')).toBeInTheDocument();
  });

  it('does not show file-scope buttons for shell.exec', () => {
    const { getByTestId, queryByTestId } = renderPrompt({
      toolName: 'shell.exec',
      argsJson: SHELL_ARGS,
    });
    fireEvent.click(getByTestId('approve-dropdown-btn'));
    expect(queryByTestId('scope-file-btn')).not.toBeInTheDocument();
    expect(queryByTestId('scope-pattern-btn')).not.toBeInTheDocument();
  });

  it('always shows ThisTool scope in dropdown', () => {
    const { getByTestId } = renderPrompt({ toolName: 'shell.exec', argsJson: SHELL_ARGS });
    fireEvent.click(getByTestId('approve-dropdown-btn'));
    expect(getByTestId('scope-tool-btn')).toBeInTheDocument();
  });
});

// ---------------------------------------------------------------------------
// Accessibility — interruptive announcement (F-088)
// ---------------------------------------------------------------------------

describe('ApprovalPrompt accessibility', () => {
  it('marks the root with role="alertdialog" and aria-live="assertive"', () => {
    const { getByTestId } = renderPrompt();
    const root = getByTestId('approval-prompt');
    expect(root).toHaveAttribute('role', 'alertdialog');
    expect(root).toHaveAttribute('aria-live', 'assertive');
  });

  it('aria-labelledby resolves to a visible non-empty title', () => {
    const { getByTestId } = renderPrompt();
    const root = getByTestId('approval-prompt');
    const labelledBy = root.getAttribute('aria-labelledby');
    expect(labelledBy).toBeTruthy();
    const title = document.getElementById(labelledBy as string);
    expect(title).not.toBeNull();
    expect(title?.textContent?.trim()).toBeTruthy();
  });

  it('uses a unique title id per toolCallId so multiple prompts can coexist', () => {
    const containerA = makeContainer();
    const containerB = makeContainer();
    render(
      () => (
        <ApprovalPrompt
          toolCallId="tc-A"
          toolName="fs.edit"
          argsJson={FS_EDIT_ARGS}
          preview={PREVIEW}
          containerRef={containerA}
          onApprove={vi.fn()}
          onReject={vi.fn()}
        />
      ),
      { container: containerA },
    );
    render(
      () => (
        <ApprovalPrompt
          toolCallId="tc-B"
          toolName="fs.edit"
          argsJson={FS_EDIT_ARGS}
          preview={PREVIEW}
          containerRef={containerB}
          onApprove={vi.fn()}
          onReject={vi.fn()}
        />
      ),
      { container: containerB },
    );
    const idA = containerA
      .querySelector('[data-testid="approval-prompt"]')
      ?.getAttribute('aria-labelledby');
    const idB = containerB
      .querySelector('[data-testid="approval-prompt"]')
      ?.getAttribute('aria-labelledby');
    expect(idA).toBeTruthy();
    expect(idB).toBeTruthy();
    expect(idA).not.toBe(idB);
  });
});

// ---------------------------------------------------------------------------
// Scope selection via buttons
// ---------------------------------------------------------------------------

describe('Scope selection — button clicks', () => {
  it('calls onApprove with Once when approve button is clicked', () => {
    const onApprove = vi.fn();
    const { getByTestId } = renderPrompt({ onApprove });
    fireEvent.click(getByTestId('approve-once-btn'));
    expect(onApprove).toHaveBeenCalledWith('Once', undefined);
  });

  it('calls onApprove with Once from scope menu', () => {
    const onApprove = vi.fn();
    const { getByTestId } = renderPrompt({ onApprove });
    fireEvent.click(getByTestId('approve-dropdown-btn'));
    fireEvent.click(getByTestId('scope-once-btn'));
    expect(onApprove).toHaveBeenCalledWith('Once', undefined);
  });

  it('calls onApprove with ThisFile from scope menu', () => {
    const onApprove = vi.fn();
    const { getByTestId } = renderPrompt({ onApprove });
    fireEvent.click(getByTestId('approve-dropdown-btn'));
    fireEvent.click(getByTestId('scope-file-btn'));
    expect(onApprove).toHaveBeenCalledWith('ThisFile', undefined);
  });

  it('opens pattern editor when ThisPattern is selected', () => {
    const { getByTestId } = renderPrompt();
    fireEvent.click(getByTestId('approve-dropdown-btn'));
    fireEvent.click(getByTestId('scope-pattern-btn'));
    expect(getByTestId('pattern-editor')).toBeInTheDocument();
    expect(getByTestId('pattern-input')).toBeInTheDocument();
  });

  it('pre-fills pattern input with directory glob', () => {
    const { getByTestId } = renderPrompt();
    fireEvent.click(getByTestId('approve-dropdown-btn'));
    fireEvent.click(getByTestId('scope-pattern-btn'));
    const input = getByTestId('pattern-input') as HTMLInputElement;
    expect(input.value).toBe('/src/*');
  });

  it('calls onApprove with ThisPattern and edited glob on confirm', () => {
    const onApprove = vi.fn();
    const { getByTestId } = renderPrompt({ onApprove });
    fireEvent.click(getByTestId('approve-dropdown-btn'));
    fireEvent.click(getByTestId('scope-pattern-btn'));
    const input = getByTestId('pattern-input') as HTMLInputElement;
    fireEvent.input(input, { target: { value: '/src/**' } });
    fireEvent.click(getByTestId('pattern-confirm-btn'));
    expect(onApprove).toHaveBeenCalledWith('ThisPattern', '/src/**');
  });

  it('cancels pattern editor without calling onApprove', () => {
    const onApprove = vi.fn();
    const { getByTestId, queryByTestId } = renderPrompt({ onApprove });
    fireEvent.click(getByTestId('approve-dropdown-btn'));
    fireEvent.click(getByTestId('scope-pattern-btn'));
    fireEvent.click(getByTestId('pattern-cancel-btn'));
    expect(queryByTestId('pattern-editor')).not.toBeInTheDocument();
    expect(onApprove).not.toHaveBeenCalled();
  });

  it('calls onApprove with ThisTool from scope menu', () => {
    const onApprove = vi.fn();
    const { getByTestId } = renderPrompt({ onApprove });
    fireEvent.click(getByTestId('approve-dropdown-btn'));
    fireEvent.click(getByTestId('scope-tool-btn'));
    expect(onApprove).toHaveBeenCalledWith('ThisTool', undefined);
  });

  it('calls onReject when reject button is clicked', () => {
    const onReject = vi.fn();
    const { getByTestId } = renderPrompt({ onReject });
    fireEvent.click(getByTestId('reject-btn'));
    expect(onReject).toHaveBeenCalledTimes(1);
  });
});

// ---------------------------------------------------------------------------
// Keyboard shortcuts
// ---------------------------------------------------------------------------

describe('Keyboard shortcuts', () => {
  it('R key calls onReject', () => {
    const onReject = vi.fn();
    const { container } = renderPrompt({ onReject });
    fireEvent.keyDown(container, { key: 'r' });
    expect(onReject).toHaveBeenCalledTimes(1);
  });

  it('Shift+R also calls onReject', () => {
    const onReject = vi.fn();
    const { container } = renderPrompt({ onReject });
    fireEvent.keyDown(container, { key: 'R' });
    expect(onReject).toHaveBeenCalledTimes(1);
  });

  it('A key calls onApprove with Once', () => {
    const onApprove = vi.fn();
    const { container } = renderPrompt({ onApprove });
    fireEvent.keyDown(container, { key: 'a' });
    expect(onApprove).toHaveBeenCalledWith('Once', undefined);
  });

  it('Enter key calls onApprove with Once', () => {
    const onApprove = vi.fn();
    const { container } = renderPrompt({ onApprove });
    fireEvent.keyDown(container, { key: 'Enter' });
    expect(onApprove).toHaveBeenCalledWith('Once', undefined);
  });

  it('F key calls onApprove with ThisFile for fs.edit', () => {
    const onApprove = vi.fn();
    const { container } = renderPrompt({ onApprove, toolName: 'fs.edit', argsJson: FS_EDIT_ARGS });
    fireEvent.keyDown(container, { key: 'f' });
    expect(onApprove).toHaveBeenCalledWith('ThisFile', undefined);
  });

  it('F key does nothing for shell.exec (no file scopes)', () => {
    const onApprove = vi.fn();
    const { container } = renderPrompt({
      onApprove,
      toolName: 'shell.exec',
      argsJson: SHELL_ARGS,
    });
    fireEvent.keyDown(container, { key: 'f' });
    expect(onApprove).not.toHaveBeenCalled();
  });

  it('P key opens pattern editor for fs.edit', () => {
    const { container, getByTestId } = renderPrompt({
      toolName: 'fs.edit',
      argsJson: FS_EDIT_ARGS,
    });
    fireEvent.keyDown(container, { key: 'p' });
    expect(getByTestId('pattern-editor')).toBeInTheDocument();
  });

  it('P key does nothing for shell.exec', () => {
    const { container, queryByTestId } = renderPrompt({
      toolName: 'shell.exec',
      argsJson: SHELL_ARGS,
    });
    fireEvent.keyDown(container, { key: 'p' });
    expect(queryByTestId('pattern-editor')).not.toBeInTheDocument();
  });

  it('T key calls onApprove with ThisTool', () => {
    const onApprove = vi.fn();
    const { container } = renderPrompt({ onApprove });
    fireEvent.keyDown(container, { key: 't' });
    expect(onApprove).toHaveBeenCalledWith('ThisTool', undefined);
  });

  it('Escape key closes pattern editor', () => {
    const { container, getByTestId, queryByTestId } = renderPrompt();
    fireEvent.keyDown(container, { key: 'p' });
    expect(getByTestId('pattern-editor')).toBeInTheDocument();
    fireEvent.keyDown(container, { key: 'Escape' });
    expect(queryByTestId('pattern-editor')).not.toBeInTheDocument();
  });

  it('shortcuts do not fire while pattern editor is open (except Escape)', () => {
    const onApprove = vi.fn();
    const { container } = renderPrompt({ onApprove });
    fireEvent.keyDown(container, { key: 'p' }); // open pattern editor
    fireEvent.keyDown(container, { key: 'a' }); // should not approve
    expect(onApprove).not.toHaveBeenCalled();
  });

  it('Escape calls onReject when pattern editor is closed', () => {
    const onReject = vi.fn();
    const { container } = renderPrompt({ onReject });
    fireEvent.keyDown(container, { key: 'Escape' });
    expect(onReject).toHaveBeenCalledTimes(1);
  });

  it('Escape does NOT call onReject while the pattern editor is open', () => {
    const onReject = vi.fn();
    const { container, getByTestId, queryByTestId } = renderPrompt({ onReject });
    fireEvent.keyDown(container, { key: 'p' });
    expect(getByTestId('pattern-editor')).toBeInTheDocument();
    fireEvent.keyDown(container, { key: 'Escape' });
    // First Escape only closes the editor
    expect(onReject).not.toHaveBeenCalled();
    expect(queryByTestId('pattern-editor')).not.toBeInTheDocument();
    // Second Escape now cancels the prompt
    fireEvent.keyDown(container, { key: 'Escape' });
    expect(onReject).toHaveBeenCalledTimes(1);
  });
});

// ---------------------------------------------------------------------------
// Focus management — initial focus, focus trap, focus restoration (F-089)
// ---------------------------------------------------------------------------

describe('Focus management — initial focus (F-089)', () => {
  it('focuses the primary Approve button on mount', () => {
    const { getByTestId } = renderPrompt();
    expect(document.activeElement).toBe(getByTestId('approve-once-btn'));
  });
});

describe('Focus management — focus trap (F-089)', () => {
  function focusableInPrompt(root: HTMLElement): HTMLElement[] {
    return Array.from(
      root.querySelectorAll<HTMLElement>(
        'button:not([disabled]), input:not([disabled]), [tabindex]:not([tabindex="-1"])',
      ),
    );
  }

  it('Tab from the last focusable element wraps to the first', () => {
    const { getByTestId } = renderPrompt();
    const root = getByTestId('approval-prompt');
    const focusables = focusableInPrompt(root);
    expect(focusables.length).toBeGreaterThan(1);
    const first = focusables[0]!;
    const last = focusables[focusables.length - 1]!;

    last.focus();
    expect(document.activeElement).toBe(last);
    fireEvent.keyDown(last, { key: 'Tab' });
    expect(document.activeElement).toBe(first);
  });

  it('Shift+Tab from the first focusable element wraps to the last', () => {
    const { getByTestId } = renderPrompt();
    const root = getByTestId('approval-prompt');
    const focusables = focusableInPrompt(root);
    expect(focusables.length).toBeGreaterThan(1);
    const first = focusables[0]!;
    const last = focusables[focusables.length - 1]!;

    first.focus();
    expect(document.activeElement).toBe(first);
    fireEvent.keyDown(first, { key: 'Tab', shiftKey: true });
    expect(document.activeElement).toBe(last);
  });

  it('Tab in the middle of the focusable set does not interfere', () => {
    const { getByTestId } = renderPrompt();
    const root = getByTestId('approval-prompt');
    const focusables = focusableInPrompt(root);
    expect(focusables.length).toBeGreaterThan(2);
    const middle = focusables[1]!;
    middle.focus();
    // No preventDefault or programmatic focus when not at a boundary —
    // we simulate the keydown but the browser's default tab order takes over.
    // The trap handler must NOT relocate focus mid-list.
    fireEvent.keyDown(middle, { key: 'Tab' });
    expect(document.activeElement).toBe(middle);
  });
});

describe('Focus management — focus restoration (F-089)', () => {
  it('restores focus to the previously focused element when the prompt unmounts', () => {
    // Trigger element lives outside the prompt's container.
    const trigger = document.createElement('button');
    trigger.textContent = 'Open approval';
    document.body.appendChild(trigger);
    trigger.focus();
    expect(document.activeElement).toBe(trigger);

    const { unmount } = renderPrompt();
    // Initial focus is captured by the prompt
    expect(document.activeElement).not.toBe(trigger);

    unmount();
    expect(document.activeElement).toBe(trigger);
  });

  it('does not throw when the previously focused element is gone on unmount', () => {
    const trigger = document.createElement('button');
    document.body.appendChild(trigger);
    trigger.focus();

    const { unmount } = renderPrompt();
    // Remove the trigger before unmount — restoration must be defensive
    trigger.remove();

    expect(() => unmount()).not.toThrow();
  });
});
