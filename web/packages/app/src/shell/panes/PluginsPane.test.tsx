import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, render } from '@solidjs/testing-library';
import { PluginsPane } from './PluginsPane';

afterEach(() => cleanup());

describe('PluginsPane', () => {
  it('renders the placeholder chrome until extensions ship', () => {
    const { getByTestId } = render(() => <PluginsPane />);
    const pane = getByTestId('workspace-pane-plugins');
    expect(pane).toBeInTheDocument();
    expect(pane.textContent ?? '').toContain('Coming soon');
  });
});
