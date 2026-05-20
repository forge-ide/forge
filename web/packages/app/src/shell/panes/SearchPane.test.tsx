import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, render } from '@solidjs/testing-library';
import { SearchPane } from './SearchPane';

afterEach(() => cleanup());

describe('SearchPane', () => {
  it('renders the placeholder chrome until the search backend lands', () => {
    const { getByTestId } = render(() => <SearchPane />);
    const pane = getByTestId('workspace-pane-search');
    expect(pane).toBeInTheDocument();
    expect(pane.textContent ?? '').toContain('Coming soon');
    expect(pane.textContent ?? '').toContain('Workspace-wide text search');
  });
});
