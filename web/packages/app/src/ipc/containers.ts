// F-597: typed wrappers for the container-lifecycle Tauri commands.
//
// Backend gates every command to the `dashboard` window label, so callers
// must originate from the Dashboard route. The wire shapes mirror the
// Rust definitions one-for-one.

import { invoke } from '../lib/tauri';

/**
 * Result of probing the container runtime. Mirrors Rust's
 * `RuntimeStatus` (`#[serde(tag = "kind")]`):
 *   - Available — runtime usable; suppress the banner.
 *   - Missing — runtime not on PATH.
 *   - Broken — installed but the probe failed (cgroup, newuidmap, SELinux…).
 *   - RootlessUnavailable — installed but rootless mode disabled.
 *   - Unknown — probe ran into an unexpected error (treat as unavailable).
 */
export type RuntimeStatus =
  | { kind: 'available' }
  | { kind: 'missing'; tool: string }
  | { kind: 'broken'; tool: string; reason: string }
  | { kind: 'rootless_unavailable'; tool: string; reason: string }
  | { kind: 'unknown'; reason: string };

/** One row of `list_active_containers`. */
export interface ContainerInfo {
  session_id: string;
  container_id: string;
  image: string;
  /** RFC-3339 timestamp captured at registration. */
  started_at: string;
  /** `true` once `stop_container` succeeded for this entry. */
  stopped: boolean;
}

/** One log line returned by `container_logs`. */
export interface LogLine {
  /** `"stdout"` or `"stderr"`. */
  stream: string;
  /** Line text with the trailing newline stripped. */
  line: string;
  /** RFC-3339 timestamp emitted by `podman logs --timestamps`. */
  timestamp?: string | null;
}

/** Probe the container runtime (podman). */
export async function detectContainerRuntime(): Promise<RuntimeStatus> {
  return invoke<RuntimeStatus>('detect_container_runtime');
}

/** Snapshot active Level-2 sandbox containers. */
export async function listActiveContainers(): Promise<ContainerInfo[]> {
  return invoke<ContainerInfo[]>('list_active_containers');
}

/** Request graceful stop of a known container. */
export async function stopContainer(containerId: string): Promise<void> {
  if (!containerId) throw new Error('stopContainer: containerId must not be empty');
  await invoke('stop_container', { containerId });
}

/** Force-remove a container (also drops it from the registry). */
export async function removeContainer(containerId: string): Promise<void> {
  if (!containerId) throw new Error('removeContainer: containerId must not be empty');
  await invoke('remove_container', { containerId });
}

/** Fetch recent log lines. `since` (RFC-3339) and `tail` are optional. */
export async function containerLogs(
  containerId: string,
  options?: { since?: string; tail?: number },
): Promise<LogLine[]> {
  if (!containerId) throw new Error('containerLogs: containerId must not be empty');
  return invoke<LogLine[]>('container_logs', {
    containerId,
    since: options?.since ?? null,
    tail: options?.tail ?? null,
  });
}

/** Tauri event name emitted by `stop`/`remove` so the dashboard refreshes. */
export const CONTAINERS_CHANGED_EVENT = 'containers:list_changed';

/** Persisted-settings key flipped by the banner's "Don't show again" button. */
export const CONTAINER_BANNER_DISMISSED_KEY = 'dashboard.container_banner_dismissed';
