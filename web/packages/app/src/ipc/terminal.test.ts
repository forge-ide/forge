import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setInvokeForTesting } from '../lib/tauri';
import {
  terminalSpawn,
  terminalWrite,
  terminalResize,
  terminalKill,
  type TerminalSpawnArgs,
} from './terminal';

describe('terminal ipc wrappers (F-365)', () => {
  let invokeMock: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    invokeMock = vi.fn();
    setInvokeForTesting(invokeMock as never);
  });

  afterEach(() => {
    setInvokeForTesting(null);
  });

  it('terminalSpawn invokes `terminal_spawn` with args struct', async () => {
    invokeMock.mockResolvedValue(undefined);

    const args: TerminalSpawnArgs = {
      terminal_id: 'abc123',
      shell: null,
      cwd: '/workspace',
      cols: 80,
      rows: 24,
    };
    await terminalSpawn(args);

    expect(invokeMock).toHaveBeenCalledWith('terminal_spawn', { args });
  });

  it('terminalWrite invokes `terminal_write` with terminalId and data', async () => {
    invokeMock.mockResolvedValue(undefined);

    await terminalWrite('abc123', [104, 101, 108, 108, 111]);

    expect(invokeMock).toHaveBeenCalledWith('terminal_write', {
      terminalId: 'abc123',
      data: [104, 101, 108, 108, 111],
    });
  });

  it('terminalResize invokes `terminal_resize` with cols and rows', async () => {
    invokeMock.mockResolvedValue(undefined);

    await terminalResize('abc123', 120, 40);

    expect(invokeMock).toHaveBeenCalledWith('terminal_resize', {
      terminalId: 'abc123',
      cols: 120,
      rows: 40,
    });
  });

  it('terminalKill invokes `terminal_kill` with terminalId', async () => {
    invokeMock.mockResolvedValue(undefined);

    await terminalKill('abc123');

    expect(invokeMock).toHaveBeenCalledWith('terminal_kill', {
      terminalId: 'abc123',
    });
  });
});
