import { type Component } from 'solid-js';
import type { ContextCategory } from './ContextPicker';
import './ContextChip.css';

// ---------------------------------------------------------------------------
// ContextChip — pill shown in the composer's `ctx-chips` row once a picker
// result is inserted (F-141). The chip is intentionally minimal: icon +
// label + dismiss-×. Full resolution (opening the file preview on hover,
// expanding a directory tree, etc.) is out of scope for F-141.
// ---------------------------------------------------------------------------

export interface ContextChipProps {
  category: ContextCategory;
  label: string;
  onDismiss: () => void;
}

function iconFor(category: ContextCategory): string {
  switch (category) {
    case 'file':
      return '[F]';
    case 'directory':
      return '[D]';
    case 'selection':
      return '[S]';
    case 'terminal':
      return '[T]';
    case 'agent':
      return '[A]';
    case 'skill':
      return '[K]';
    case 'url':
      return '[U]';
  }
}

export const ContextChip: Component<ContextChipProps> = (props) => {
  return (
    <span
      class="ctx-chip"
      data-testid="ctx-chip"
      data-category={props.category}
    >
      <span class="ctx-chip__icon" aria-hidden="true">
        {iconFor(props.category)}
      </span>
      <span class="ctx-chip__label">{props.label}</span>
      <button
        type="button"
        class="ctx-chip__dismiss"
        data-testid="ctx-chip-dismiss"
        aria-label={`Remove ${props.label}`}
        onClick={() => props.onDismiss()}
      >
        ×
      </button>
    </span>
  );
};
