import { describe, expect, it, vi, beforeEach } from 'vitest';
import { render, fireEvent } from '@solidjs/testing-library';
import { WhitelistedPill } from './WhitelistedPill';

beforeEach(() => {
  while (document.body.firstChild) {
    document.body.removeChild(document.body.firstChild);
  }
});

describe('WhitelistedPill — session level', () => {
  it('renders the pill with the label and no provenance suffix', () => {
    const { getByTestId } = render(() => (
      <WhitelistedPill label="this file" level="session" onRevoke={vi.fn()} />
    ));
    const pill = getByTestId('whitelisted-pill');
    expect(pill).toBeInTheDocument();
    expect(pill).toHaveTextContent('whitelisted · this file');
    // No trailing "· workspace"/"· user" for session-level entries.
    expect(pill.textContent?.trim()).toBe('whitelisted · this file');
  });

  it('revoke button text mentions this session', () => {
    const { getByTestId } = render(() => (
      <WhitelistedPill label="this tool" level="session" onRevoke={vi.fn()} />
    ));
    fireEvent.click(getByTestId('whitelisted-pill'));
    expect(getByTestId('revoke-btn')).toHaveTextContent(/this session/i);
  });
});

describe('WhitelistedPill — workspace level', () => {
  it('renders with " · workspace" provenance suffix', () => {
    const { getByTestId } = render(() => (
      <WhitelistedPill label="this file" level="workspace" onRevoke={vi.fn()} />
    ));
    expect(getByTestId('whitelisted-pill')).toHaveTextContent(
      'whitelisted · this file · workspace',
    );
  });

  it('revoke button text mentions this workspace', () => {
    const { getByTestId } = render(() => (
      <WhitelistedPill label="this tool" level="workspace" onRevoke={vi.fn()} />
    ));
    fireEvent.click(getByTestId('whitelisted-pill'));
    expect(getByTestId('revoke-btn')).toHaveTextContent(/this workspace/i);
  });
});

describe('WhitelistedPill — user level', () => {
  it('renders with " · user" provenance suffix', () => {
    const { getByTestId } = render(() => (
      <WhitelistedPill label="this tool" level="user" onRevoke={vi.fn()} />
    ));
    expect(getByTestId('whitelisted-pill')).toHaveTextContent(
      'whitelisted · this tool · user',
    );
  });

  it('revoke button text mentions this user', () => {
    const { getByTestId } = render(() => (
      <WhitelistedPill label="this tool" level="user" onRevoke={vi.fn()} />
    ));
    fireEvent.click(getByTestId('whitelisted-pill'));
    expect(getByTestId('revoke-btn')).toHaveTextContent(/this user/i);
  });
});

describe('WhitelistedPill — popover behavior', () => {
  it('does not show popover initially', () => {
    const { queryByTestId } = render(() => (
      <WhitelistedPill label="this tool" level="session" onRevoke={vi.fn()} />
    ));
    expect(queryByTestId('whitelist-popover')).not.toBeInTheDocument();
  });

  it('opens popover on pill click', () => {
    const { getByTestId } = render(() => (
      <WhitelistedPill label="this file" level="session" onRevoke={vi.fn()} />
    ));
    fireEvent.click(getByTestId('whitelisted-pill'));
    expect(getByTestId('whitelist-popover')).toBeInTheDocument();
  });

  it('closes popover on second click', () => {
    const { getByTestId, queryByTestId } = render(() => (
      <WhitelistedPill label="this file" level="session" onRevoke={vi.fn()} />
    ));
    fireEvent.click(getByTestId('whitelisted-pill'));
    fireEvent.click(getByTestId('whitelisted-pill'));
    expect(queryByTestId('whitelist-popover')).not.toBeInTheDocument();
  });

  it('calls onRevoke when revoke button is clicked', () => {
    const onRevoke = vi.fn();
    const { getByTestId } = render(() => (
      <WhitelistedPill label="this file" level="workspace" onRevoke={onRevoke} />
    ));
    fireEvent.click(getByTestId('whitelisted-pill'));
    fireEvent.click(getByTestId('revoke-btn'));
    expect(onRevoke).toHaveBeenCalledTimes(1);
  });

  it('closes popover after revoke', () => {
    const { getByTestId, queryByTestId } = render(() => (
      <WhitelistedPill label="this file" level="user" onRevoke={vi.fn()} />
    ));
    fireEvent.click(getByTestId('whitelisted-pill'));
    fireEvent.click(getByTestId('revoke-btn'));
    expect(queryByTestId('whitelist-popover')).not.toBeInTheDocument();
  });
});
