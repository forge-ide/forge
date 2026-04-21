// F-138: StatusBar + BgAgentsBadge component tests.
//
// The badge listens to the F-137 `BackgroundAgentStarted` /
// `BackgroundAgentCompleted` events (forwarded onto `session:event`) to flip
// its running-count and fire the notification configured in
// `settings.notifications.bg_agents`. The tests drive the component through
// injectable seams:
//
//  - `listBackgroundAgents` prop → initial snapshot, post-reconnect recovery.
//  - `promoteBackgroundAgent` / `stopBackgroundAgent` props → popover actions.
//  - `subscribe` prop → the IPC bus handler that fires `started` / `completed`.
//  - `notificationAdapter` prop → `{ toast, os }` doubles so we never touch
//    the real `@tauri-apps/plugin-notification` or the global `pushToast`
//    queue in jsdom.
//
// The DoD pins:
//   - badge renders "N bg" when count > 0
//   - click opens a popover listing running agents + Promote/Stop
//   - Promote/Stop call their respective IPCs with the right args
//   - completion event fires the configured notification
//     (toast | os | both | silent)

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library';
import type { BgAgentSummary, SessionId } from '@forge/ipc';
import { StatusBar } from './StatusBar';
import type {
  BgAgentsSubscribe,
  NotificationAdapter,
} from './StatusBar';
import type { SessionEventPayload } from '../ipc/session';
import { resetSettingsStore, seedSettings } from '../stores/settings';

const SID = 'test-session-1' as SessionId;

function running(id: string, name = 'writer'): BgAgentSummary {
  return { id, agent_name: name, state: 'Running' };
}

/**
 * Build a controllable fake subscribe surface so tests can drive the
 * StatusBar through `background_agent_*` events without a real Tauri bus.
 * The StatusBar calls `subscribe(handler)` once on mount; the returned
 * unlisten is invoked on cleanup.
 */
interface FakeBus {
  subscribe: BgAgentsSubscribe;
  emit: (payload: SessionEventPayload) => void;
  unlistenCalls: number;
}

function fakeBus(): FakeBus {
  let handler: ((payload: SessionEventPayload) => void) | null = null;
  let unlistenCalls = 0;
  const bus: FakeBus = {
    subscribe: async (h) => {
      handler = h;
      return () => {
        unlistenCalls += 1;
        handler = null;
      };
    },
    emit: (payload) => {
      if (handler) handler(payload);
    },
    unlistenCalls: 0,
  };
  // `unlistenCalls` reads through a getter so tests observe the live count.
  Object.defineProperty(bus, 'unlistenCalls', {
    get: () => unlistenCalls,
  });
  return bus;
}

function fakeNotifier(): NotificationAdapter & {
  toastCalls: string[];
  osCalls: string[];
  permission: 'granted' | 'denied' | 'prompt';
  requestCalls: number;
} {
  const toastCalls: string[] = [];
  const osCalls: string[] = [];
  let permission: 'granted' | 'denied' | 'prompt' = 'granted';
  let requestCalls = 0;
  const adapter = {
    toast: (msg: string) => {
      toastCalls.push(msg);
    },
    os: async (msg: string) => {
      osCalls.push(msg);
    },
    isPermissionGranted: async () => permission === 'granted',
    requestPermission: async () => {
      requestCalls += 1;
      if (permission === 'prompt') permission = 'granted';
      return permission === 'granted' ? 'granted' : 'denied';
    },
  } as NotificationAdapter;

  return new Proxy(adapter, {
    get(target, prop) {
      switch (prop) {
        case 'toastCalls':
          return toastCalls;
        case 'osCalls':
          return osCalls;
        case 'permission':
          return permission;
        case 'requestCalls':
          return requestCalls;
        default:
          return Reflect.get(target, prop);
      }
    },
    set(_target, prop, value) {
      if (prop === 'permission') permission = value;
      return true;
    },
  }) as NotificationAdapter & {
    toastCalls: string[];
    osCalls: string[];
    permission: 'granted' | 'denied' | 'prompt';
    requestCalls: number;
  };
}

beforeEach(() => {
  resetSettingsStore();
});

afterEach(() => cleanup());

describe('StatusBar — badge visibility + count', () => {
  it('hides the badge when there are zero running agents', async () => {
    const bus = fakeBus();
    const list = vi.fn().mockResolvedValue([]);
    const { queryByTestId } = render(() => (
      <StatusBar
        sessionId={SID}
        listBackgroundAgents={list}
        subscribe={bus.subscribe}
      />
    ));
    await waitFor(() => {
      expect(list).toHaveBeenCalledWith(SID);
    });
    expect(queryByTestId('bg-agents-badge')).toBeNull();
  });

  it('renders "N bg" when initial list returns running agents', async () => {
    const bus = fakeBus();
    const list = vi
      .fn()
      .mockResolvedValue([running('a1'), running('a2', 'reviewer')]);
    const { findByTestId } = render(() => (
      <StatusBar
        sessionId={SID}
        listBackgroundAgents={list}
        subscribe={bus.subscribe}
      />
    ));
    const badge = await findByTestId('bg-agents-badge');
    expect(badge).toHaveTextContent('2 bg');
  });

  it('increments on BackgroundAgentStarted and decrements on BackgroundAgentCompleted', async () => {
    const bus = fakeBus();
    const list = vi.fn().mockResolvedValue([running('a1')]);
    const { findByTestId, queryByTestId } = render(() => (
      <StatusBar
        sessionId={SID}
        listBackgroundAgents={list}
        subscribe={bus.subscribe}
      />
    ));
    const badge = await findByTestId('bg-agents-badge');
    expect(badge).toHaveTextContent('1 bg');

    // Another started event — badge flips to 2.
    bus.emit({
      session_id: SID,
      seq: 1,
      event: { type: 'background_agent_started', id: 'a2', agent: 'x', at: 'now' },
    });
    await waitFor(() => {
      expect(badge).toHaveTextContent('2 bg');
    });

    // Completion for a1 — badge back to 1.
    bus.emit({
      session_id: SID,
      seq: 2,
      event: { type: 'background_agent_completed', id: 'a1', at: 'now' },
    });
    await waitFor(() => {
      expect(badge).toHaveTextContent('1 bg');
    });

    // Final completion — badge hidden.
    bus.emit({
      session_id: SID,
      seq: 3,
      event: { type: 'background_agent_completed', id: 'a2', at: 'now' },
    });
    await waitFor(() => {
      expect(queryByTestId('bg-agents-badge')).toBeNull();
    });
  });

  it('ignores events for other sessions', async () => {
    const bus = fakeBus();
    const list = vi.fn().mockResolvedValue([running('a1')]);
    const { findByTestId } = render(() => (
      <StatusBar
        sessionId={SID}
        listBackgroundAgents={list}
        subscribe={bus.subscribe}
      />
    ));
    const badge = await findByTestId('bg-agents-badge');
    expect(badge).toHaveTextContent('1 bg');

    bus.emit({
      session_id: 'other-session',
      seq: 1,
      event: { type: 'background_agent_started', id: 'zz', agent: 'x', at: 'now' },
    });

    // Give the handler a turn to reject the event.
    await Promise.resolve();
    expect(badge).toHaveTextContent('1 bg');
  });
});

describe('StatusBar — popover interaction', () => {
  it('opens a popover listing running agents when the badge is clicked', async () => {
    const bus = fakeBus();
    const list = vi
      .fn()
      .mockResolvedValue([running('a1', 'writer'), running('a2', 'reviewer')]);
    const { findByTestId, queryByTestId } = render(() => (
      <StatusBar
        sessionId={SID}
        listBackgroundAgents={list}
        subscribe={bus.subscribe}
      />
    ));
    const badge = await findByTestId('bg-agents-badge');
    expect(queryByTestId('bg-agents-popover')).toBeNull();

    fireEvent.click(badge);
    const popover = await findByTestId('bg-agents-popover');
    expect(popover).toHaveTextContent('writer');
    expect(popover).toHaveTextContent('reviewer');
  });

  it('Promote button calls promoteBackgroundAgent with the right id', async () => {
    const bus = fakeBus();
    const list = vi.fn().mockResolvedValue([running('abcd1234', 'writer')]);
    const promote = vi.fn().mockResolvedValue(undefined);
    const stop = vi.fn().mockResolvedValue(undefined);
    const { findByTestId } = render(() => (
      <StatusBar
        sessionId={SID}
        listBackgroundAgents={list}
        subscribe={bus.subscribe}
        promoteBackgroundAgent={promote}
        stopBackgroundAgent={stop}
      />
    ));
    fireEvent.click(await findByTestId('bg-agents-badge'));
    fireEvent.click(await findByTestId('bg-agents-promote-abcd1234'));
    await waitFor(() => {
      expect(promote).toHaveBeenCalledWith(SID, 'abcd1234');
    });
    expect(stop).not.toHaveBeenCalled();
  });

  it('Stop button calls stopBackgroundAgent with the right id', async () => {
    const bus = fakeBus();
    const list = vi.fn().mockResolvedValue([running('a1', 'writer')]);
    const promote = vi.fn().mockResolvedValue(undefined);
    const stop = vi.fn().mockResolvedValue(undefined);
    const { findByTestId } = render(() => (
      <StatusBar
        sessionId={SID}
        listBackgroundAgents={list}
        subscribe={bus.subscribe}
        promoteBackgroundAgent={promote}
        stopBackgroundAgent={stop}
      />
    ));
    fireEvent.click(await findByTestId('bg-agents-badge'));
    fireEvent.click(await findByTestId('bg-agents-stop-a1'));
    await waitFor(() => {
      expect(stop).toHaveBeenCalledWith(SID, 'a1');
    });
  });
});

describe('StatusBar — notification modes', () => {
  async function primeWithCompletion(
    mode: 'toast' | 'os' | 'both' | 'silent',
    opts: {
      notifier: NotificationAdapter;
    },
  ) {
    seedSettings({
      notifications: { bg_agents: mode },
      windows: { session_mode: 'single' },
    });
    const bus = fakeBus();
    const list = vi.fn().mockResolvedValue([running('a1', 'writer')]);
    const result = render(() => (
      <StatusBar
        sessionId={SID}
        listBackgroundAgents={list}
        subscribe={bus.subscribe}
        notificationAdapter={opts.notifier}
      />
    ));
    await waitFor(() => {
      expect(list).toHaveBeenCalled();
    });
    bus.emit({
      session_id: SID,
      seq: 1,
      event: { type: 'background_agent_completed', id: 'a1', at: 'now' },
    });
    return result;
  }

  it('mode=toast pushes an in-app toast only', async () => {
    const notifier = fakeNotifier();
    await primeWithCompletion('toast', { notifier });
    await waitFor(() => {
      expect(
        (notifier as unknown as { toastCalls: string[] }).toastCalls.length,
      ).toBe(1);
    });
    expect(
      (notifier as unknown as { osCalls: string[] }).osCalls,
    ).toEqual([]);
  });

  it('mode=os fires an OS notification only', async () => {
    const notifier = fakeNotifier();
    await primeWithCompletion('os', { notifier });
    await waitFor(() => {
      expect(
        (notifier as unknown as { osCalls: string[] }).osCalls.length,
      ).toBe(1);
    });
    expect(
      (notifier as unknown as { toastCalls: string[] }).toastCalls,
    ).toEqual([]);
  });

  it('mode=both fires both toast and OS notification', async () => {
    const notifier = fakeNotifier();
    await primeWithCompletion('both', { notifier });
    await waitFor(() => {
      expect(
        (notifier as unknown as { toastCalls: string[] }).toastCalls.length,
      ).toBe(1);
    });
    expect(
      (notifier as unknown as { osCalls: string[] }).osCalls.length,
    ).toBe(1);
  });

  it('mode=silent does nothing', async () => {
    const notifier = fakeNotifier();
    await primeWithCompletion('silent', { notifier });
    // Let the listener handler run.
    await Promise.resolve();
    await Promise.resolve();
    expect(
      (notifier as unknown as { toastCalls: string[] }).toastCalls,
    ).toEqual([]);
    expect(
      (notifier as unknown as { osCalls: string[] }).osCalls,
    ).toEqual([]);
  });

  it('requests OS permission before firing when not already granted', async () => {
    const notifier = fakeNotifier();
    (notifier as unknown as { permission: string }).permission = 'prompt';
    await primeWithCompletion('os', { notifier });
    await waitFor(() => {
      expect(
        (notifier as unknown as { requestCalls: number }).requestCalls,
      ).toBe(1);
    });
    await waitFor(() => {
      expect(
        (notifier as unknown as { osCalls: string[] }).osCalls.length,
      ).toBe(1);
    });
  });
});

describe('StatusBar — cleanup', () => {
  it('unsubscribes from the event bus on unmount', async () => {
    const bus = fakeBus();
    const list = vi.fn().mockResolvedValue([]);
    const { unmount } = render(() => (
      <StatusBar
        sessionId={SID}
        listBackgroundAgents={list}
        subscribe={bus.subscribe}
      />
    ));
    await waitFor(() => {
      expect(list).toHaveBeenCalled();
    });
    expect(bus.unlistenCalls).toBe(0);
    unmount();
    expect(bus.unlistenCalls).toBe(1);
  });
});
