import { describe, expect, it } from 'vitest';
import { usageRange, setUsageRange, usageReport, setUsageReport } from './usage';

describe('usage store', () => {
  it('defaults to a last-7-days range with no report', () => {
    expect(usageRange()).toEqual({ kind: 'last_days', days: 7 });
    expect(usageReport()).toBeNull();
  });

  it('setUsageRange updates the range signal', () => {
    setUsageRange({ kind: 'last_days', days: 30 });
    expect(usageRange()).toEqual({ kind: 'last_days', days: 30 });
    setUsageRange({ kind: 'last_days', days: 7 });
  });

  it('setUsageReport accepts a report object', () => {
    setUsageReport({ total_tokens: 0 });
    expect(usageReport()).toEqual({ total_tokens: 0 });
    setUsageReport(null);
  });
});
