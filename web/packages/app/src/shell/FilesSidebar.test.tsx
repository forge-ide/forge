// F-126: FilesSidebar render + context-menu behavior.
//
// Tree-load is injected via `loadTree` so tests run without Tauri. The
// component is expected to:
//  1. Render the children of the loaded root (not the root itself — the
//     header owns the workspace label slot).
//  2. Toggle directory expansion on click; render recursive children when
//     open.
//  3. Surface a context menu with Open/Rename/Delete on right-click.
//  4. Route Open → `props.onOpen(path)`.
//  5. Route Rename → `props.renamePath(sessionId, from, to)` with the
//     new path computed by swapping the leaf name.
//  6. Route Delete → `props.deletePath(sessionId, path)`.
//  7. Display gitignore-filtered trees correctly (the filtering itself is
//     exercised in the Rust integration test; here we just confirm the
//     component renders whatever the tree call returns).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library';
import type { SessionId, TreeNodeDto, TreeStatsDto } from '@forge/ipc';
import { FilesSidebar } from './FilesSidebar';
import { setActiveSessionId } from '../stores/session';

const SID = 'session-sidebar-test' as SessionId;
const WS = '/workspace/demo';

function node(
  name: string,
  path: string,
  kind: 'File' | 'Dir',
  children?: TreeNodeDto[],
  stats?: TreeStatsDto | null,
): TreeNodeDto {
  return {
    name,
    path,
    kind,
    children: kind === 'Dir' ? children ?? [] : null,
    stats,
  } as TreeNodeDto;
}

function demoTree(): TreeNodeDto {
  return node(
    'demo',
    WS,
    'Dir',
    [
      node('README.md', `${WS}/README.md`, 'File'),
      node('src', `${WS}/src`, 'Dir', [
        node('main.ts', `${WS}/src/main.ts`, 'File'),
      ]),
    ],
  );
}

function treeWithoutIgnored(): TreeNodeDto {
  // Simulates the post-gitignore-filter tree: `node_modules/` and `*.log`
  // are absent because the shell-side `tree` command now uses the
  // gitignored walker. The component renders whatever it receives — the
  // key invariant is that gitignored entries never appear.
  return node(
    'demo',
    WS,
    'Dir',
    [
      node('app.ts', `${WS}/app.ts`, 'File'),
      node('keep.md', `${WS}/keep.md`, 'File'),
    ],
  );
}

// F-385: FilesSidebar reads the session id from the global `activeSessionId`
// signal. Tests mount outside of SessionWindow, so we seed the signal
// directly before each mount and clear it in afterEach.
beforeEach(() => {
  setActiveSessionId(SID);
});

afterEach(() => {
  setActiveSessionId(null);
  cleanup();
});

describe('FilesSidebar render', () => {
  it('renders EXPLORER header and the root children on mount', async () => {
    const loadTree = vi.fn().mockResolvedValue(demoTree());
    const { findByText, getByText } = render(() => (
      <FilesSidebar
        workspaceRoot={WS}
        onOpen={vi.fn()}
        loadTree={loadTree}
      />
    ));
    await findByText('README.md');
    expect(getByText('EXPLORER')).toBeInTheDocument();
    expect(getByText('src')).toBeInTheDocument();
    expect(loadTree).toHaveBeenCalledWith(SID, WS);
  });

  it('expands and collapses a directory on click', async () => {
    const loadTree = vi.fn().mockResolvedValue(demoTree());
    const { findByText, queryByText } = render(() => (
      <FilesSidebar
        workspaceRoot={WS}
        onOpen={vi.fn()}
        loadTree={loadTree}
      />
    ));
    const srcRow = await findByText('src');
    // Not expanded yet — main.ts is hidden.
    expect(queryByText('main.ts')).toBeNull();
    fireEvent.click(srcRow);
    await waitFor(() => expect(queryByText('main.ts')).not.toBeNull());
    fireEvent.click(srcRow);
    await waitFor(() => expect(queryByText('main.ts')).toBeNull());
  });

  it('opens a file on double-click', async () => {
    const loadTree = vi.fn().mockResolvedValue(demoTree());
    const onOpen = vi.fn();
    const { findByText } = render(() => (
      <FilesSidebar
        workspaceRoot={WS}
        onOpen={onOpen}
        loadTree={loadTree}
      />
    ));
    const readme = await findByText('README.md');
    fireEvent.dblClick(readme);
    expect(onOpen).toHaveBeenCalledWith(`${WS}/README.md`);
  });

  it('renders only the gitignore-filtered tree the IPC layer returns', async () => {
    const loadTree = vi.fn().mockResolvedValue(treeWithoutIgnored());
    const { findByText, queryByText } = render(() => (
      <FilesSidebar
        workspaceRoot={WS}
        onOpen={vi.fn()}
        loadTree={loadTree}
      />
    ));
    await findByText('app.ts');
    expect(queryByText('node_modules')).toBeNull();
    expect(queryByText('err.log')).toBeNull();
  });

  it('surfaces a load error in an alert region', async () => {
    const loadTree = vi
      .fn()
      .mockRejectedValue(new Error('tauri invoke unavailable (command=tree)'));
    const { findByTestId } = render(() => (
      <FilesSidebar
        workspaceRoot={WS}
        onOpen={vi.fn()}
        loadTree={loadTree}
      />
    ));
    const err = await findByTestId('files-sidebar-error');
    expect(err.textContent).toMatch(/tauri invoke unavailable/);
  });
});

describe('FilesSidebar context menu', () => {
  it('opens a context menu on right-click with Open/Rename/Delete', async () => {
    const loadTree = vi.fn().mockResolvedValue(demoTree());
    const { findByText, findByTestId } = render(() => (
      <FilesSidebar
        workspaceRoot={WS}
        onOpen={vi.fn()}
        loadTree={loadTree}
      />
    ));
    const readme = await findByText('README.md');
    fireEvent.contextMenu(readme);
    const menu = await findByTestId('files-sidebar-menu');
    expect(menu).toBeInTheDocument();
    expect(await findByTestId('files-sidebar-menu-open')).toBeInTheDocument();
    expect(await findByTestId('files-sidebar-menu-rename')).toBeInTheDocument();
    expect(await findByTestId('files-sidebar-menu-delete')).toBeInTheDocument();
  });

  it('routes Open via context menu through props.onOpen', async () => {
    const loadTree = vi.fn().mockResolvedValue(demoTree());
    const onOpen = vi.fn();
    const { findByText, findByTestId } = render(() => (
      <FilesSidebar
        workspaceRoot={WS}
        onOpen={onOpen}
        loadTree={loadTree}
      />
    ));
    const readme = await findByText('README.md');
    fireEvent.contextMenu(readme);
    fireEvent.click(await findByTestId('files-sidebar-menu-open'));
    expect(onOpen).toHaveBeenCalledWith(`${WS}/README.md`);
  });

  it('routes Rename through the rename_path wrapper with the new absolute path', async () => {
    const loadTree = vi.fn().mockResolvedValue(demoTree());
    const renamePath = vi.fn().mockResolvedValue(undefined);
    const { findByText, findByTestId } = render(() => (
      <FilesSidebar
        workspaceRoot={WS}
        onOpen={vi.fn()}
        loadTree={loadTree}
        renamePath={renamePath}
        promptForRename={() => 'renamed.md'}
      />
    ));
    const readme = await findByText('README.md');
    fireEvent.contextMenu(readme);
    const rename = await findByTestId('files-sidebar-menu-rename');
    fireEvent.click(rename);
    await waitFor(() =>
      expect(renamePath).toHaveBeenCalledWith(
        SID,
        `${WS}/README.md`,
        `${WS}/renamed.md`,
      ),
    );
    // Tree reloads on success.
    expect(loadTree).toHaveBeenCalledTimes(2);
  });

  it('skips rename when the prompt returns null (cancel)', async () => {
    const loadTree = vi.fn().mockResolvedValue(demoTree());
    const renamePath = vi.fn().mockResolvedValue(undefined);
    const { findByText, findByTestId } = render(() => (
      <FilesSidebar
        workspaceRoot={WS}
        onOpen={vi.fn()}
        loadTree={loadTree}
        renamePath={renamePath}
        promptForRename={() => null}
      />
    ));
    const readme = await findByText('README.md');
    fireEvent.contextMenu(readme);
    fireEvent.click(await findByTestId('files-sidebar-menu-rename'));
    expect(renamePath).not.toHaveBeenCalled();
  });

  it('routes Delete through the delete_path wrapper on confirm', async () => {
    const loadTree = vi.fn().mockResolvedValue(demoTree());
    const deletePath = vi.fn().mockResolvedValue(undefined);
    const { findByText, findByTestId } = render(() => (
      <FilesSidebar
        workspaceRoot={WS}
        onOpen={vi.fn()}
        loadTree={loadTree}
        deletePath={deletePath}
        confirmDelete={() => true}
      />
    ));
    const readme = await findByText('README.md');
    fireEvent.contextMenu(readme);
    fireEvent.click(await findByTestId('files-sidebar-menu-delete'));
    await waitFor(() =>
      expect(deletePath).toHaveBeenCalledWith(SID, `${WS}/README.md`),
    );
    expect(loadTree).toHaveBeenCalledTimes(2);
  });

  it('skips delete when the confirm returns false', async () => {
    const loadTree = vi.fn().mockResolvedValue(demoTree());
    const deletePath = vi.fn().mockResolvedValue(undefined);
    const { findByText, findByTestId } = render(() => (
      <FilesSidebar
        workspaceRoot={WS}
        onOpen={vi.fn()}
        loadTree={loadTree}
        deletePath={deletePath}
        confirmDelete={() => false}
      />
    ));
    const readme = await findByText('README.md');
    fireEvent.contextMenu(readme);
    fireEvent.click(await findByTestId('files-sidebar-menu-delete'));
    expect(deletePath).not.toHaveBeenCalled();
  });

  // F-385: single source of truth for sessionId. Panes read it from the
  // global `activeSessionId` signal; no `sessionId` prop is accepted. The
  // test flips the signal mid-render and asserts the next refresh uses the
  // new id — proving the component reads the signal live, not a prop
  // captured at mount.
  it('uses the live activeSessionId signal when calling loadTree (no sessionId prop)', async () => {
    const loadTree = vi.fn().mockResolvedValue(demoTree());
    const { findByText } = render(() => (
      <FilesSidebar
        workspaceRoot={WS}
        onOpen={vi.fn()}
        loadTree={loadTree}
      />
    ));
    await findByText('README.md');
    expect(loadTree).toHaveBeenCalledWith(SID, WS);
    // Flip the signal and trigger a refresh via the header button; the next
    // load call must carry the new session id.
    const nextSid = 'session-sidebar-next' as SessionId;
    setActiveSessionId(nextSid);
    const refreshBtn = document.querySelector(
      '.files-sidebar__refresh',
    ) as HTMLButtonElement | null;
    expect(refreshBtn).not.toBeNull();
    loadTree.mockClear();
    refreshBtn!.click();
    await waitFor(() => expect(loadTree).toHaveBeenCalledWith(nextSid, WS));
  });

  // F-411 (V8): context-menu items are buttons executing an action on the
  // selected tree row — per voice-terminology.md §8, they carry verb(+noun)
  // labels in display caps as literal text so screen readers announce them
  // uppercase.
  it('context-menu items carry OPEN / RENAME / DELETE as literal text', async () => {
    const loadTree = vi.fn().mockResolvedValue(demoTree());
    const { findByText, findByTestId } = render(() => (
      <FilesSidebar
        workspaceRoot={WS}
        onOpen={vi.fn()}
        loadTree={loadTree}
      />
    ));
    const readme = await findByText('README.md');
    fireEvent.contextMenu(readme);
    expect((await findByTestId('files-sidebar-menu-open')).textContent).toContain('OPEN');
    expect((await findByTestId('files-sidebar-menu-rename')).textContent).toContain('RENAME');
    expect((await findByTestId('files-sidebar-menu-delete')).textContent).toContain('DELETE');
  });
});

describe('FilesSidebar stats notice (F-536)', () => {
  it('shows "N files not shown" when stats.truncated and omitted_count > 0', async () => {
    const truncatedRoot = node('demo', WS, 'Dir', [
      node('README.md', `${WS}/README.md`, 'File'),
    ], { truncated: true, omitted_count: 42, error_count: 0 });
    const loadTree = vi.fn().mockResolvedValue(truncatedRoot);
    const { findByTestId } = render(() => (
      <FilesSidebar
        workspaceRoot={WS}
        onOpen={vi.fn()}
        loadTree={loadTree}
      />
    ));
    const notice = await findByTestId('files-sidebar-stats-notice');
    expect(notice.textContent).toMatch(/42 files not shown/);
  });

  it('shows "1 file not shown" (singular) when omitted_count is 1', async () => {
    const truncatedRoot = node('demo', WS, 'Dir', [], {
      truncated: true,
      omitted_count: 1,
      error_count: 0,
    });
    const loadTree = vi.fn().mockResolvedValue(truncatedRoot);
    const { findByTestId } = render(() => (
      <FilesSidebar
        workspaceRoot={WS}
        onOpen={vi.fn()}
        loadTree={loadTree}
      />
    ));
    const notice = await findByTestId('files-sidebar-stats-notice');
    expect(notice.textContent).toMatch(/1 file not shown/);
  });

  it('shows error count when error_count > 0', async () => {
    const errorRoot = node('demo', WS, 'Dir', [
      node('app.ts', `${WS}/app.ts`, 'File'),
    ], { truncated: false, omitted_count: 0, error_count: 3 });
    const loadTree = vi.fn().mockResolvedValue(errorRoot);
    const { findByTestId } = render(() => (
      <FilesSidebar
        workspaceRoot={WS}
        onOpen={vi.fn()}
        loadTree={loadTree}
      />
    ));
    const notice = await findByTestId('files-sidebar-stats-notice');
    expect(notice.textContent).toMatch(/3 read errors/);
  });

  it('combines truncation and error messages with a separator', async () => {
    const combinedRoot = node('demo', WS, 'Dir', [], {
      truncated: true,
      omitted_count: 10,
      error_count: 2,
    });
    const loadTree = vi.fn().mockResolvedValue(combinedRoot);
    const { findByTestId } = render(() => (
      <FilesSidebar
        workspaceRoot={WS}
        onOpen={vi.fn()}
        loadTree={loadTree}
      />
    ));
    const notice = await findByTestId('files-sidebar-stats-notice');
    expect(notice.textContent).toMatch(/10 files not shown/);
    expect(notice.textContent).toMatch(/2 read errors/);
  });

  it('renders no stats notice when stats is absent', async () => {
    const loadTree = vi.fn().mockResolvedValue(demoTree());
    const { findByText, queryByTestId } = render(() => (
      <FilesSidebar
        workspaceRoot={WS}
        onOpen={vi.fn()}
        loadTree={loadTree}
      />
    ));
    await findByText('README.md');
    expect(queryByTestId('files-sidebar-stats-notice')).toBeNull();
  });

  it('renders no stats notice when stats shows no truncation and no errors', async () => {
    const cleanRoot = node('demo', WS, 'Dir', [
      node('README.md', `${WS}/README.md`, 'File'),
    ], { truncated: false, omitted_count: 0, error_count: 0 });
    const loadTree = vi.fn().mockResolvedValue(cleanRoot);
    const { findByText, queryByTestId } = render(() => (
      <FilesSidebar
        workspaceRoot={WS}
        onOpen={vi.fn()}
        loadTree={loadTree}
      />
    ));
    await findByText('README.md');
    expect(queryByTestId('files-sidebar-stats-notice')).toBeNull();
  });
});

describe('FilesSidebar loading and empty states (F-400)', () => {
  it('shows LOADING TREE while the first loadTree call is pending', async () => {
    let resolve!: (v: TreeNodeDto) => void;
    const loadTree = vi.fn().mockReturnValue(
      new Promise<TreeNodeDto>((res) => {
        resolve = res;
      }),
    );
    const { getByTestId } = render(() => (
      <FilesSidebar
        sessionId={SID}
        workspaceRoot={WS}
        onOpen={vi.fn()}
        loadTree={loadTree}
      />
    ));
    const loading = getByTestId('files-sidebar-loading');
    expect(loading).toBeInTheDocument();
    expect(loading.getAttribute('role')).toBe('status');
    expect(loading.textContent).toContain('LOADING TREE');

    resolve(demoTree());
    await Promise.resolve();
    await Promise.resolve();
  });

  it('removes LOADING TREE after loadTree resolves', async () => {
    const loadTree = vi.fn().mockResolvedValue(demoTree());
    const { findByText, queryByTestId } = render(() => (
      <FilesSidebar
        sessionId={SID}
        workspaceRoot={WS}
        onOpen={vi.fn()}
        loadTree={loadTree}
      />
    ));
    await findByText('README.md');
    expect(queryByTestId('files-sidebar-loading')).toBeNull();
  });

  it('shows NO FILES when the workspace is empty (loaded, no error)', async () => {
    const emptyRoot: TreeNodeDto = {
      name: 'empty',
      path: WS,
      kind: 'Dir',
      children: [],
      stats: null,
    } as unknown as TreeNodeDto;
    const loadTree = vi.fn().mockResolvedValue(emptyRoot);
    const { findByTestId } = render(() => (
      <FilesSidebar
        sessionId={SID}
        workspaceRoot={WS}
        onOpen={vi.fn()}
        loadTree={loadTree}
      />
    ));
    const empty = await findByTestId('files-sidebar-empty');
    expect(empty).toBeInTheDocument();
    expect(empty.getAttribute('role')).toBe('status');
    expect(empty.textContent).toContain('NO FILES');
  });

  it('does not show NO FILES while still loading', async () => {
    let resolve!: (v: TreeNodeDto) => void;
    const loadTree = vi.fn().mockReturnValue(
      new Promise<TreeNodeDto>((res) => {
        resolve = res;
      }),
    );
    const { queryByTestId } = render(() => (
      <FilesSidebar
        sessionId={SID}
        workspaceRoot={WS}
        onOpen={vi.fn()}
        loadTree={loadTree}
      />
    ));
    // Still loading — empty placeholder must not appear
    expect(queryByTestId('files-sidebar-empty')).toBeNull();
    expect(queryByTestId('files-sidebar-loading')).toBeInTheDocument();

    resolve(demoTree());
    await Promise.resolve();
    await Promise.resolve();
  });

  it('does not show NO FILES when there is a load error', async () => {
    const loadTree = vi.fn().mockRejectedValue(new Error('permission denied'));
    const { findByTestId, queryByTestId } = render(() => (
      <FilesSidebar
        sessionId={SID}
        workspaceRoot={WS}
        onOpen={vi.fn()}
        loadTree={loadTree}
      />
    ));
    await findByTestId('files-sidebar-error');
    expect(queryByTestId('files-sidebar-empty')).toBeNull();
  });
});
