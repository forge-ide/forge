import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { setInvokeForTesting } from '../lib/tauri';
import {
  SESSION_WIDE_SCOPE,
  listAgents,
  listMcpServers,
  listProvidersRoster,
  listSkills,
} from './catalog';

const invokeMock = vi.fn();

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue([]);
  setInvokeForTesting(invokeMock as never);
});

afterEach(() => {
  setInvokeForTesting(null);
});

describe('catalog IPC wrappers (F-592)', () => {
  it('listSkills invokes list_skills with workspaceRoot + SessionWide scope by default', async () => {
    await listSkills('/ws');
    expect(invokeMock).toHaveBeenCalledWith('list_skills', {
      workspaceRoot: '/ws',
      scope: { type: 'SessionWide' },
    });
  });

  it('listMcpServers invokes list_mcp_servers (NOT session_list_mcp_servers)', async () => {
    await listMcpServers('/ws');
    expect(invokeMock).toHaveBeenCalledWith('list_mcp_servers', {
      workspaceRoot: '/ws',
      scope: SESSION_WIDE_SCOPE,
    });
  });

  it('listAgents invokes list_agents with the supplied scope', async () => {
    const scope = { type: 'Agent' as const, id: 'refactor-bot' };
    await listAgents('/ws', scope);
    expect(invokeMock).toHaveBeenCalledWith('list_agents', {
      workspaceRoot: '/ws',
      scope,
    });
  });

  it('listProvidersRoster invokes list_providers (catalog) — not dashboard_list_providers', async () => {
    await listProvidersRoster('/ws');
    expect(invokeMock).toHaveBeenCalledWith('list_providers', {
      workspaceRoot: '/ws',
      scope: SESSION_WIDE_SCOPE,
    });
  });

  it('surfaces the IPC rejection', async () => {
    invokeMock.mockRejectedValueOnce(new Error('not in registry'));
    await expect(listSkills('/bogus')).rejects.toThrow('not in registry');
  });
});
