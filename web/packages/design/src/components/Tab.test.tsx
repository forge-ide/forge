import { describe, expect, it, vi } from 'vitest';
import { render, fireEvent, cleanup } from '@solidjs/testing-library';
import { Tabs, Tab } from './Tab';

describe('Tabs container', () => {
  it('renders role="tablist" by default', () => {
    const { container } = render(() => (
      <Tabs>
        <Tab selected={true}>One</Tab>
        <Tab selected={false}>Two</Tab>
      </Tabs>
    ));
    expect(container.querySelector('[role=tablist]')).toBeTruthy();
    cleanup();
  });

  it('renders role="radiogroup" when variant="radio"', () => {
    const { container } = render(() => (
      <Tabs variant="radio">
        <Tab variant="radio" selected={true}>
          A
        </Tab>
      </Tabs>
    ));
    expect(container.querySelector('[role=radiogroup]')).toBeTruthy();
    cleanup();
  });
});

describe('Tab', () => {
  it('renders role="tab" + aria-selected when variant=tab', () => {
    const { getByRole } = render(() => <Tab selected={true}>One</Tab>);
    const tab = getByRole('tab');
    expect(tab.getAttribute('aria-selected')).toBe('true');
    expect(tab.hasAttribute('aria-checked')).toBe(false);
    cleanup();
  });

  it('renders role="radio" + aria-checked when variant=radio', () => {
    const { getByRole } = render(() => (
      <Tab variant="radio" selected={true}>
        Pick
      </Tab>
    ));
    const radio = getByRole('radio');
    expect(radio.getAttribute('aria-checked')).toBe('true');
    expect(radio.hasAttribute('aria-selected')).toBe(false);
    cleanup();
  });

  it('selected tab is in the tab order; unselected gets tabindex=-1', () => {
    const { getByText } = render(() => (
      <Tabs>
        <Tab selected={true}>One</Tab>
        <Tab selected={false}>Two</Tab>
      </Tabs>
    ));
    expect(getByText('One').getAttribute('tabindex')).toBe('0');
    expect(getByText('Two').getAttribute('tabindex')).toBe('-1');
    cleanup();
  });

  it('forwards onClick', () => {
    const onClick = vi.fn();
    const { getByRole } = render(() => (
      <Tab selected={false} onClick={onClick}>
        Hit
      </Tab>
    ));
    fireEvent.click(getByRole('tab'));
    expect(onClick).toHaveBeenCalledTimes(1);
    cleanup();
  });

  it('renders type="button" so a parent form is never submitted', () => {
    const { getByRole } = render(() => <Tab selected={true}>One</Tab>);
    expect(getByRole('tab').getAttribute('type')).toBe('button');
    cleanup();
  });

  it('renders badgeCount when supplied', () => {
    const { container } = render(() => (
      <Tab selected={true} badgeCount={3}>
        Active
      </Tab>
    ));
    expect(container.querySelector('.forge-tab__badge')?.textContent).toBe('3');
    cleanup();
  });
});
