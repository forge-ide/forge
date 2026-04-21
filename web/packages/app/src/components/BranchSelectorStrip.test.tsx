import { describe, expect, it, vi } from 'vitest';
import { render, fireEvent } from '@solidjs/testing-library';
import { BranchSelectorStrip } from './BranchSelectorStrip';

describe('BranchSelectorStrip', () => {
  it('renders position/total label', () => {
    const { getByTestId } = render(() => (
      <BranchSelectorStrip
        position={2}
        total={3}
        onPrev={() => {}}
        onNext={() => {}}
        onToggleInfo={() => {}}
        infoOpen={false}
      />
    ));
    expect(getByTestId('branch-strip-label').textContent).toBe('variant 2 of 3');
  });

  it('fires onPrev when the left arrow is clicked', () => {
    const onPrev = vi.fn();
    const { getByTestId } = render(() => (
      <BranchSelectorStrip
        position={2}
        total={3}
        onPrev={onPrev}
        onNext={() => {}}
        onToggleInfo={() => {}}
        infoOpen={false}
      />
    ));
    fireEvent.click(getByTestId('branch-strip-prev'));
    expect(onPrev).toHaveBeenCalledTimes(1);
  });

  it('fires onNext when the right arrow is clicked', () => {
    const onNext = vi.fn();
    const { getByTestId } = render(() => (
      <BranchSelectorStrip
        position={1}
        total={3}
        onPrev={() => {}}
        onNext={onNext}
        onToggleInfo={() => {}}
        infoOpen={false}
      />
    ));
    fireEvent.click(getByTestId('branch-strip-next'));
    expect(onNext).toHaveBeenCalledTimes(1);
  });

  it('ArrowLeft on the strip triggers onPrev', () => {
    const onPrev = vi.fn();
    const { getByTestId } = render(() => (
      <BranchSelectorStrip
        position={2}
        total={3}
        onPrev={onPrev}
        onNext={() => {}}
        onToggleInfo={() => {}}
        infoOpen={false}
      />
    ));
    const strip = getByTestId('branch-selector-strip');
    fireEvent.keyDown(strip, { key: 'ArrowLeft' });
    expect(onPrev).toHaveBeenCalledTimes(1);
  });

  it('ArrowRight on the strip triggers onNext', () => {
    const onNext = vi.fn();
    const { getByTestId } = render(() => (
      <BranchSelectorStrip
        position={2}
        total={3}
        onPrev={() => {}}
        onNext={onNext}
        onToggleInfo={() => {}}
        infoOpen={false}
      />
    ));
    const strip = getByTestId('branch-selector-strip');
    fireEvent.keyDown(strip, { key: 'ArrowRight' });
    expect(onNext).toHaveBeenCalledTimes(1);
  });

  it('info button toggles via onToggleInfo and reflects aria-expanded from infoOpen', () => {
    const onToggleInfo = vi.fn();
    const { getByTestId } = render(() => (
      <BranchSelectorStrip
        position={1}
        total={2}
        onPrev={() => {}}
        onNext={() => {}}
        onToggleInfo={onToggleInfo}
        infoOpen={true}
      />
    ));
    const info = getByTestId('branch-strip-info');
    expect(info.getAttribute('aria-expanded')).toBe('true');
    fireEvent.click(info);
    expect(onToggleInfo).toHaveBeenCalledTimes(1);
  });
});
