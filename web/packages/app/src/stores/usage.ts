import { createSignal } from 'solid-js';

export type UsageRange = { kind: 'last_days'; days: number };

export interface UsageReport {
  total_tokens: number;
}

export const [usageRange, setUsageRange] = createSignal<UsageRange>({ kind: 'last_days', days: 7 });
export const [usageReport, setUsageReport] = createSignal<UsageReport | null>(null);
