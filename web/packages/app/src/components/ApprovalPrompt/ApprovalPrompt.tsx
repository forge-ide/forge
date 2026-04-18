import {
  type Component,
  createSignal,
  Show,
  onMount,
  onCleanup,
} from 'solid-js';
import type { ApprovalScope } from '@forge/ipc';
import type { ApprovalPreview } from '../../stores/messages';
import { defaultPatternForPath } from '../../stores/approvals';
import './ApprovalPrompt.css';

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

export interface ApprovalPromptProps {
  toolCallId: string;
  toolName: string;
  argsJson: string;
  preview: ApprovalPreview;
  /** Container element to attach keyboard listeners to (the card div). */
  containerRef: HTMLElement;
  onApprove: (scope: ApprovalScope, pattern?: string) => void;
  onReject: () => void;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function extractPath(argsJson: string): string {
  try {
    const args = JSON.parse(argsJson) as Record<string, unknown>;
    return typeof args['path'] === 'string' ? args['path'] : '';
  } catch {
    return '';
  }
}

function isFileTool(toolName: string): boolean {
  return toolName === 'fs.edit' || toolName === 'fs.write';
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export const ApprovalPrompt: Component<ApprovalPromptProps> = (props) => {
  const [menuOpen, setMenuOpen] = createSignal(false);
  const [patternMode, setPatternMode] = createSignal(false);
  const [pattern, setPattern] = createSignal('');

  const path = () => extractPath(props.argsJson);
  const showFileScopes = () => isFileTool(props.toolName) && path() !== '';

  const openPatternEditor = () => {
    setPattern(defaultPatternForPath(path()));
    setMenuOpen(false);
    setPatternMode(true);
  };

  const approveWithScope = (scope: ApprovalScope, pat?: string) => {
    setMenuOpen(false);
    setPatternMode(false);
    props.onApprove(scope, pat);
  };

  const handleKeyDown = (e: KeyboardEvent) => {
    // Don't intercept if user is editing pattern input
    if (patternMode()) {
      if (e.key === 'Escape') {
        e.preventDefault();
        setPatternMode(false);
      }
      return;
    }
    if (e.key === 'r' || e.key === 'R') {
      e.preventDefault();
      props.onReject();
    } else if (e.key === 'a' || e.key === 'A' || e.key === 'Enter') {
      e.preventDefault();
      approveWithScope('Once');
    } else if ((e.key === 'f' || e.key === 'F') && showFileScopes()) {
      e.preventDefault();
      approveWithScope('ThisFile');
    } else if ((e.key === 'p' || e.key === 'P') && showFileScopes()) {
      e.preventDefault();
      openPatternEditor();
    } else if (e.key === 't' || e.key === 'T') {
      e.preventDefault();
      approveWithScope('ThisTool');
    }
  };

  onMount(() => {
    props.containerRef.addEventListener('keydown', handleKeyDown);
  });

  onCleanup(() => {
    props.containerRef.removeEventListener('keydown', handleKeyDown);
  });

  return (
    <div class="approval-prompt" data-testid="approval-prompt">
      {/* Preview */}
      <div class="approval-prompt__preview" data-testid="approval-preview">
        <pre class="approval-prompt__preview-text">{props.preview.description}</pre>
      </div>

      {/* Pattern editor */}
      <Show when={patternMode()}>
        <div class="approval-prompt__pattern-editor" data-testid="pattern-editor">
          <label class="approval-prompt__pattern-label" for="approval-pattern-input">
            Approve this pattern:
          </label>
          <div class="approval-prompt__pattern-row">
            <input
              id="approval-pattern-input"
              class="approval-prompt__pattern-input"
              data-testid="pattern-input"
              type="text"
              value={pattern()}
              onInput={(e) => setPattern(e.currentTarget.value)}
            />
            <button
              type="button"
              class="approval-prompt__btn approval-prompt__btn--primary"
              data-testid="pattern-confirm-btn"
              onClick={() => approveWithScope('ThisPattern', pattern())}
            >
              Confirm
            </button>
            <button
              type="button"
              class="approval-prompt__btn approval-prompt__btn--ghost"
              data-testid="pattern-cancel-btn"
              onClick={() => setPatternMode(false)}
            >
              Cancel
            </button>
          </div>
        </div>
      </Show>

      {/* Actions */}
      <Show when={!patternMode()}>
        <div class="approval-prompt__actions">
          <button
            type="button"
            class="approval-prompt__btn approval-prompt__btn--ghost"
            data-testid="reject-btn"
            onClick={() => props.onReject()}
          >
            Reject
            <kbd class="approval-prompt__kbd">R</kbd>
          </button>

          <div class="approval-prompt__approve-group">
            {/* Primary approve button — Once (default) */}
            <button
              type="button"
              class="approval-prompt__btn approval-prompt__btn--primary"
              data-testid="approve-once-btn"
              onClick={() => approveWithScope('Once')}
            >
              Approve
              <kbd class="approval-prompt__kbd">A</kbd>
            </button>

            {/* Dropdown toggle */}
            <button
              type="button"
              class="approval-prompt__btn approval-prompt__btn--primary approval-prompt__dropdown-toggle"
              data-testid="approve-dropdown-btn"
              aria-label="More approval scopes"
              aria-expanded={menuOpen()}
              onClick={() => setMenuOpen((v) => !v)}
            >
              ▾
            </button>

            {/* Scope menu */}
            <Show when={menuOpen()}>
              <div class="approval-prompt__menu" data-testid="scope-menu" role="menu">
                <button
                  type="button"
                  class="approval-prompt__menu-item"
                  data-testid="scope-once-btn"
                  role="menuitem"
                  onClick={() => approveWithScope('Once')}
                >
                  Once
                  <kbd class="approval-prompt__kbd">A</kbd>
                </button>

                <Show when={showFileScopes()}>
                  <button
                    type="button"
                    class="approval-prompt__menu-item"
                    data-testid="scope-file-btn"
                    role="menuitem"
                    onClick={() => approveWithScope('ThisFile')}
                  >
                    This file
                    <kbd class="approval-prompt__kbd">F</kbd>
                  </button>

                  <button
                    type="button"
                    class="approval-prompt__menu-item"
                    data-testid="scope-pattern-btn"
                    role="menuitem"
                    onClick={openPatternEditor}
                  >
                    This pattern
                    <kbd class="approval-prompt__kbd">P</kbd>
                  </button>
                </Show>

                <button
                  type="button"
                  class="approval-prompt__menu-item"
                  data-testid="scope-tool-btn"
                  role="menuitem"
                  onClick={() => approveWithScope('ThisTool')}
                >
                  This tool
                  <kbd class="approval-prompt__kbd">T</kbd>
                </button>
              </div>
            </Show>
          </div>
        </div>
      </Show>
    </div>
  );
};
