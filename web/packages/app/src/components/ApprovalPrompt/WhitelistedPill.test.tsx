import { describe, expect, it, vi, beforeEach } from 'vitest';
import { render, fireEvent } from '@solidjs/testing-library';
import { WhitelistedPill } from './WhitelistedPill';

beforeEach(() => {
  while (document.body.firstChild) {
    document.body.removeChild(document.body.firstChild);
  }
});

describe('WhitelistedPill', () => {
  it('renders the pill with the label', () => {
    const { getByTestId } = render(() => (
      <WhitelistedPill label="this file" onRevoke={vi.fn()} />
    ));
    const pill = getByTestId('whitelisted-pill');
    expect(pill).toBeInTheDocument();
    expect(pill).toHaveTextContent('whitelisted · this file');
  });

  it('does not show popover initially', () => {
    const { queryByTestId } = render(() => (
      <WhitelistedPill label="this tool" onRevoke={vi.fn()} />
    ));
    expect(queryByTestId('whitelist-popover')).not.toBeInTheDocument();
  });

  it('opens popover on pill click', () => {
    const { getByTestId } = render(() => (
      <WhitelistedPill label="this file" onRevoke={vi.fn()} />
    ));
    fireEvent.click(getByTestId('whitelisted-pill'));
    expect(getByTestId('whitelist-popover')).toBeInTheDocument();
  });

  it('closes popover on second click', () => {
    const { getByTestId, queryByTestId } = render(() => (
      <WhitelistedPill label="this file" onRevoke={vi.fn()} />
    ));
    fireEvent.click(getByTestId('whitelisted-pill'));
    fireEvent.click(getByTestId('whitelisted-pill'));
    expect(queryByTestId('whitelist-popover')).not.toBeInTheDocument();
  });

  it('calls onRevoke when revoke button is clicked', () => {
    const onRevoke = vi.fn();
    const { getByTestId } = render(() => (
      <WhitelistedPill label="this file" onRevoke={onRevoke} />
    ));
    fireEvent.click(getByTestId('whitelisted-pill'));
    fireEvent.click(getByTestId('revoke-btn'));
    expect(onRevoke).toHaveBeenCalledTimes(1);
  });

  it('closes popover after revoke', () => {
    const { getByTestId, queryByTestId } = render(() => (
      <WhitelistedPill label="this file" onRevoke={vi.fn()} />
    ));
    fireEvent.click(getByTestId('whitelisted-pill'));
    fireEvent.click(getByTestId('revoke-btn'));
    expect(queryByTestId('whitelist-popover')).not.toBeInTheDocument();
  });
});
