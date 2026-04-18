import { describe, expect, it } from 'vitest';
import type { ProviderId } from '@forge/ipc';
import {
  providers,
  setProviders,
  mcpServers,
  skills,
  agents,
  containers,
} from './catalog';

describe('catalog store', () => {
  it('all stores are initially empty arrays', () => {
    expect(providers).toEqual([]);
    expect(mcpServers).toEqual([]);
    expect(skills).toEqual([]);
    expect(agents).toEqual([]);
    expect(containers).toEqual([]);
  });

  it('setProviders accepts entries typed against ipc ProviderId', () => {
    const p: { id: ProviderId; name: string } = { id: 'anthropic' as ProviderId, name: 'Anthropic' };
    setProviders([p]);
    expect(providers).toHaveLength(1);
    expect(providers[0]?.id).toBe('anthropic');
    setProviders([]);
  });
});
