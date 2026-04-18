import { describe, expect, it } from 'vitest';
import { render } from '@solidjs/testing-library';
import { Dashboard } from './Dashboard';

describe('Dashboard', () => {
  it('renders the placeholder heading', () => {
    const { getByRole } = render(() => <Dashboard />);
    const heading = getByRole('heading', { level: 1 });
    expect(heading.textContent).toBe('Forge — Dashboard');
  });
});
