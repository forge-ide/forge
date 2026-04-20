import { invoke } from '../lib/tauri';
import type { Layouts } from '@forge/ipc';

/**
 * F-120: thin wrappers around the Tauri `read_layouts` / `write_layouts`
 * commands. The shell backs the two calls with `<workspaceRoot>/.forge/layouts.json`
 * and silently degrades to the default single-pane layout on a missing or
 * corrupt file — consumers should treat a resolved `Layouts` as authoritative
 * and not branch on "did the file exist". Writes surface disk errors directly;
 * the debouncer in `layoutStore` retries on the next change.
 */
export async function readLayouts(workspaceRoot: string): Promise<Layouts> {
  return invoke<Layouts>('read_layouts', { workspaceRoot });
}

export async function writeLayouts(
  workspaceRoot: string,
  layouts: Layouts,
): Promise<void> {
  await invoke('write_layouts', { workspaceRoot, layouts });
}
