import { describe, expect, it, vi } from 'vitest';
import { render } from '@solidjs/testing-library';
import { PaneHeader } from './PaneHeader';

// PaneHeader close button (F-084): per voice-terminology.md §8 "Button labels:
// verb + noun in display caps." The close button's visible text must read
// `CLOSE SESSION` literally — the aria-label remains a sentence for AT users.

describe('PaneHeader close button — voice & terminology (F-084)', () => {
  it('renders close button text as `CLOSE SESSION` (verb + noun, display caps)', () => {
    const { getByRole } = render(() => (
      <PaneHeader
        subject="example"
        providerLabel="local"
        costLabel="$0.00"
        onClose={vi.fn()}
      />
    ));
    const btn = getByRole('button', { name: 'Close session window' });
    // Literal match — case matters; this is the source-of-truth string,
    // not a CSS uppercase transform.
    expect(btn.textContent).toBe('CLOSE SESSION');
  });

  it('keeps the existing aria-label as a sentence for screen readers', () => {
    const { getByRole } = render(() => (
      <PaneHeader
        subject="example"
        providerLabel="local"
        costLabel="$0.00"
        onClose={vi.fn()}
      />
    ));
    const btn = getByRole('button', { name: 'Close session window' });
    expect(btn.getAttribute('aria-label')).toBe('Close session window');
  });
});
