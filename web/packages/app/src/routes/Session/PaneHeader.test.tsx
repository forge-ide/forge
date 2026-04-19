import { describe, expect, it, vi } from 'vitest';
import { render } from '@solidjs/testing-library';
import type { ProviderId } from '@forge/ipc';
import { PaneHeader } from './PaneHeader';

// PaneHeader close button (F-084): per voice-terminology.md §8 "Button labels:
// verb + noun in display caps." The close button's visible text must read
// `CLOSE SESSION` literally — the aria-label remains a sentence for AT users.

describe('PaneHeader close button — voice & terminology (F-084)', () => {
  it('renders close button text as `CLOSE SESSION` (verb + noun, display caps)', () => {
    const { getByRole } = render(() => (
      <PaneHeader
        subject="example"
        providerId={'ollama' as ProviderId}
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
        providerId={'ollama' as ProviderId}
        providerLabel="local"
        costLabel="$0.00"
        onClose={vi.fn()}
      />
    ));
    const btn = getByRole('button', { name: 'Close session window' });
    expect(btn.getAttribute('aria-label')).toBe('Close session window');
  });
});

// PaneHeader provider pill (F-091): per ai-patterns.md §7, the provider pill
// color must follow the active provider — anthropic/ember, openai/amber,
// ollama/lm-studio/local/steel, otherwise iron-200. The component plumbs the
// active provider's accent into a CSS custom property on the provider pill so
// the existing class-driven CSS picks it up without per-provider rules.

describe('PaneHeader provider pill accent (F-091)', () => {
  it.each<[ProviderId, string]>([
    ['anthropic' as ProviderId, 'var(--color-provider-anthropic)'],
    ['openai' as ProviderId, 'var(--color-provider-openai)'],
    ['ollama' as ProviderId, 'var(--color-provider-local)'],
    ['local' as ProviderId, 'var(--color-provider-local)'],
    ['lm-studio' as ProviderId, 'var(--color-provider-local)'],
    ['custom' as ProviderId, 'var(--color-provider-custom)'],
  ])('binds the pill accent to %s → %s', (providerId, expectedAccent) => {
    const { getByTestId } = render(() => (
      <PaneHeader
        subject="example"
        providerId={providerId}
        providerLabel={String(providerId)}
        costLabel="$0.00"
        onClose={vi.fn()}
      />
    ));
    const pill = getByTestId('pane-header-provider');
    // Inline style carries the CSS variable so the rule stays generic.
    // jsdom's getPropertyValue is reliable for inline custom properties.
    expect(pill.style.getPropertyValue('--pane-header-provider-accent')).toBe(
      expectedAccent,
    );
  });

  it('falls back to the custom accent for an unknown provider id', () => {
    const { getByTestId } = render(() => (
      <PaneHeader
        subject="example"
        providerId={'made-up-provider' as ProviderId}
        providerLabel="custom"
        costLabel="$0.00"
        onClose={vi.fn()}
      />
    ));
    const pill = getByTestId('pane-header-provider');
    expect(pill.style.getPropertyValue('--pane-header-provider-accent')).toBe(
      'var(--color-provider-custom)',
    );
  });
});
