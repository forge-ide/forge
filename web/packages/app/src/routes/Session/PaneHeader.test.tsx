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

// PaneHeader compactness (F-119): per layout-panes.md §3.7 + pane-header.md
// §2.3, the header must collapse chrome as the pane narrows. The prop is
// optional — omitting it is `full`. At `compact` the type label collapses
// to an icon; at `icon-only` the provider pill + cost meter also hide.
// Callers derive the prop from `usePaneWidth` against the enclosing pane.

describe('PaneHeader compactness (F-119)', () => {
  it('defaults to `full` when no compactness prop is passed — all chrome visible', () => {
    const { getByTestId, queryByTestId } = render(() => (
      <PaneHeader
        subject="example"
        providerId={'ollama' as ProviderId}
        providerLabel="local"
        costLabel="$0.00"
        onClose={vi.fn()}
      />
    ));
    const root = getByTestId('pane-header-subject').parentElement;
    expect(root?.getAttribute('data-compactness')).toBe('full');
    expect(queryByTestId('pane-header-provider')).not.toBeNull();
    expect(queryByTestId('pane-header-cost')).not.toBeNull();
  });

  it('at `compact`: marks root with data-compactness="compact" and collapses the type label to an icon', () => {
    const { getByTestId, queryByTestId } = render(() => (
      <PaneHeader
        subject="example"
        providerId={'ollama' as ProviderId}
        providerLabel="local"
        costLabel="$0.00"
        compactness="compact"
        onClose={vi.fn()}
      />
    ));
    const root = getByTestId('pane-header-subject').parentElement;
    expect(root?.getAttribute('data-compactness')).toBe('compact');
    // Type label swapped to an icon glyph (aria-hidden; the a11y name is
    // carried by the pane-header-type-label element via aria-label).
    const typeLabel = getByTestId('pane-header-type-label');
    expect(typeLabel.getAttribute('data-icon-only')).toBe('true');
    // Badges (provider pill + cost meter) still render at `compact` — they
    // only disappear at `icon-only`. This matches DoD: labels collapse at
    // 320px, badges collapse at 240px.
    expect(queryByTestId('pane-header-provider')).not.toBeNull();
    expect(queryByTestId('pane-header-cost')).not.toBeNull();
  });

  it('at `icon-only`: hides both the provider pill and the cost meter badges', () => {
    const { getByTestId, queryByTestId } = render(() => (
      <PaneHeader
        subject="example"
        providerId={'ollama' as ProviderId}
        providerLabel="local"
        costLabel="$0.00"
        compactness="icon-only"
        onClose={vi.fn()}
      />
    ));
    const root = getByTestId('pane-header-subject').parentElement;
    expect(root?.getAttribute('data-compactness')).toBe('icon-only');
    // Label is still in icon mode at `icon-only` (`compact` or narrower).
    const typeLabel = getByTestId('pane-header-type-label');
    expect(typeLabel.getAttribute('data-icon-only')).toBe('true');
    // Badges removed from the tree entirely — the narrow-width spec treats
    // them as non-essential chrome. Keeping them with `display: none`
    // would leave them discoverable by screen readers.
    expect(queryByTestId('pane-header-provider')).toBeNull();
    expect(queryByTestId('pane-header-cost')).toBeNull();
  });

  it('keeps the close button visible at every compactness level', () => {
    for (const c of ['full', 'compact', 'icon-only'] as const) {
      const { getByRole, unmount } = render(() => (
        <PaneHeader
          subject="example"
          providerId={'ollama' as ProviderId}
          providerLabel="local"
          costLabel="$0.00"
          compactness={c}
          onClose={vi.fn()}
        />
      ));
      // Close is essential at every width — no pane is useful without a
      // way to dismiss it. (Per pane-header.md §PH.6 the close button stays.)
      expect(getByRole('button', { name: 'Close session window' })).not.toBeNull();
      unmount();
    }
  });
});
