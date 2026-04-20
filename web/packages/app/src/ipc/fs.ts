// F-122: typed wrappers over the `read_file`, `write_file`, and `tree`
// Tauri commands. All three are session-scoped (authz'd via
// `require_window_label` on the shell side). The workspace root is NOT
// passed from the webview — the shell looks it up server-side from the
// cache populated at `session_hello` time. A webview cannot widen its
// sandbox by lying about the workspace root because the command never
// reads a webview-supplied value. See `crates/forge-shell/src/ipc.rs`
// F-122 block for the authority.

import { invoke } from '../lib/tauri';
import type { FileContent, SessionId, TreeNodeDto } from '@forge/ipc';

export type { FileContent, TreeNodeDto };

/** Read the file at `path`. `path` must resolve inside the session's
 *  cached workspace root (server-side enforced). */
export async function readFile(
  sessionId: SessionId,
  path: string,
): Promise<FileContent> {
  return invoke<FileContent>('read_file', {
    sessionId,
    path,
  });
}

/**
 * Write `bytes` atomically to `path`. The shell sends the vector over the
 * Tauri IPC channel as a JSON number array; the sender converts a UTF-8
 * string into the number array so the common "save Monaco buffer" call
 * site stays ergonomic. For binary writes, pass a `Uint8Array` directly.
 *
 * `path` must resolve inside the session's cached workspace root; the
 * `forge-fs` allowlist enforces that server-side — a lying webview cannot
 * widen the sandbox.
 */
export async function writeFile(
  sessionId: SessionId,
  path: string,
  content: string | Uint8Array,
): Promise<void> {
  const bytes =
    typeof content === 'string'
      ? Array.from(new TextEncoder().encode(content))
      : Array.from(content);
  await invoke('write_file', {
    sessionId,
    path,
    bytes,
  });
}

/**
 * Walk the tree rooted at `root`. Pass the session's workspace root as
 * `root` to list the whole workspace. `depth` defaults to 6 on the shell
 * side and is capped at 16. Directories beyond the depth appear with
 * `children: []`; entries past the in-crate entry budget are silently
 * truncated (see `forge-fs::tree`).
 */
export async function tree(
  sessionId: SessionId,
  root: string,
  depth?: number,
): Promise<TreeNodeDto> {
  return invoke<TreeNodeDto>('tree', {
    sessionId,
    root,
    depth,
  });
}
