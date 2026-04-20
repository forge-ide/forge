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

import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library';
import type { SessionId, TreeNodeDto } from '@forge/ipc';
import { FilesSidebar } from './FilesSidebar';

const SID = 'session-sidebar-test' as SessionId;
const WS = '/workspace/demo';

function node(
  name: string,
  path: string,
  kind: 'File' | 'Dir',
  children?: TreeNodeDto[],
): TreeNodeDto {
  return {
    name,
    path,
    kind,
    children: kind === 'Dir' ? children ?? [] : null,
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

afterEach(() => cleanup());

describe('FilesSidebar render', () => {
  it('renders EXPLORER header and the root children on mount', async () => {
    const loadTree = vi.fn().mockResolvedValue(demoTree());
    const { findByText, getByText } = render(() => (
      <FilesSidebar
        sessionId={SID}
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
        sessionId={SID}
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
        sessionId={SID}
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
        sessionId={SID}
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
        sessionId={SID}
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
        sessionId={SID}
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
        sessionId={SID}
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
        sessionId={SID}
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
        sessionId={SID}
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
        sessionId={SID}
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
        sessionId={SID}
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
});
