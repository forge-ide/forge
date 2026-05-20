import {
  type Component,
  type JSX,
  Match,
  Show,
  Switch,
  createSignal,
  onCleanup,
  onMount,
} from 'solid-js';
import { useMatch, useNavigate } from '@solidjs/router';
import { ActivityBar, type ActivityId } from './ActivityBar';
import { FilesSidebar } from './FilesSidebar';
import { Sidebar } from './Sidebar';
import { StatusBar } from './StatusBar';
import { ChatPane } from './panes/ChatPane';
import { AgentsPane } from './panes/AgentsPane';
import { SkillsPane } from './panes/SkillsPane';
import { McpPane } from './panes/McpPane';
import { SearchPane } from './panes/SearchPane';
import { PluginsPane } from './panes/PluginsPane';
import { activeOpenFile, activeWorkspaceRoot } from '../stores/session';
import { CommandPalette, registerBuiltins } from '../commands';
import { ToastHost } from '../components/ToastHost';
import { createMountedSubscription } from '../ipc/useEventListener';
import { listen } from '@tauri-apps/api/event';
import './AppShell.css';

/**
 * Route-agnostic chrome. Mounts the StatusBar on every top-level route,
 * the dashboard Sidebar on dashboard routes, and the workspace ActivityBar
 * + the matching sidebar pane on the session/workspace route. Sits at the
 * Router `root` so navigating between routes never unmounts the chrome.
 *
 * The session window is conceptually a workspace window: sessions belong
 * to the workspace, and the ActivityBar's seven panes surface the
 * workspace's facets (files, search, sessions list, agents, skills, MCP
 * servers, plugins). The underlying Tauri label stays `session-*` for now
 * — multi-session-in-one-window is a follow-up that requires a window-
 * label refactor.
 */
export const AppShell: Component<{ children?: JSX.Element }> = (props) => {
  // Register built-in palette entries once the Router context is available
  // — AppShell is the first child of `<Router>`.
  registerBuiltins();

  // The activity bar's selection drives the sidebar pane slot. Only
  // mounted on the session/workspace route; dashboard routes use the
  // dashboard Sidebar instead.
  const sessionMatch = useMatch(() => '/session/*');
  const isSessionRoute = () => sessionMatch() !== undefined;
  const navigate = useNavigate();

  // Workspace-per-window event: the shell's WindowManager fires
  // `workspace:navigate` with a session id whenever `session_start` or
  // `open_session` targets an already-open workspace window. The
  // listener routes the in-window URL to the requested session so the
  // user sees the new session without a window swap.
  createMountedSubscription(() =>
    listen<string>('workspace:navigate', (e) => {
      const sessionId = e.payload;
      if (typeof sessionId === 'string' && sessionId.length > 0) {
        navigate(`/session/${sessionId}`);
      }
    }),
  );

  const [activeActivity, setActiveActivity] = createSignal<ActivityId | null>(null);

  const onActivitySelect = (id: ActivityId): void => {
    // Clicking the active entry closes the pane (toggle); clicking any
    // other entry switches the open pane to it.
    setActiveActivity((prev) => (prev === id ? null : id));
  };

  // Cmd/Ctrl+Shift+E mirrors the activity-bar Files toggle so the shortcut
  // works regardless of which surface holds keyboard focus.
  const onShortcut = (e: KeyboardEvent): void => {
    const mod = e.metaKey || e.ctrlKey;
    if (mod && e.shiftKey && (e.key === 'E' || e.key === 'e')) {
      e.preventDefault();
      onActivitySelect('files');
    }
  };

  onMount(() => {
    window.addEventListener('keydown', onShortcut);
  });
  onCleanup(() => {
    window.removeEventListener('keydown', onShortcut);
  });

  /** Files pane requires a workspace; every other pane either has its
   *  own empty state or accepts a `null` workspace and renders the
   *  no-workspace branch internally. */
  const showFilesPane = (): boolean =>
    activeActivity() === 'files' && activeWorkspaceRoot() !== null;

  const onFileOpen = (path: string): void => {
    const handler = activeOpenFile();
    if (handler === null) {
      console.warn('openFile dropped — no active session bridge', path);
      return;
    }
    handler(path);
  };

  return (
    <div class="app-shell" data-testid="app-shell">
      <div class="app-shell__body">
        <Show when={isSessionRoute()}>
          <ActivityBar
            active={activeActivity()}
            onSelect={onActivitySelect}
          />
        </Show>
        {/* Primary nav is the dashboard window's chrome — its entries
            navigate to dashboard routes (/, /providers, /catalog/*,
            /usage) whose IPCs are dashboard-only and would reject a
            session-* webview label. Hide it in session windows so the
            user neither sees disabled-feeling links nor triggers
            dashboard list IPCs (sessionList, listProviders, ...) that
            would noisily fail with "forbidden: window label mismatch". */}
        <Show when={!isSessionRoute()}>
          <Sidebar />
        </Show>
        {/* Sidebar pane slot — exactly one pane visible at a time, driven
            by `activeActivity()`. Only mounted on the session/workspace
            route, mirroring the ActivityBar's gating. */}
        <Show when={isSessionRoute() && activeActivity() !== null}>
          <Switch>
            <Match when={showFilesPane()}>
              <FilesSidebar
                workspaceRoot={activeWorkspaceRoot() as string}
                onOpen={onFileOpen}
              />
            </Match>
            <Match when={activeActivity() === 'search'}>
              <SearchPane />
            </Match>
            <Match when={activeActivity() === 'chat'}>
              <ChatPane workspaceRoot={activeWorkspaceRoot()} />
            </Match>
            <Match when={activeActivity() === 'agents'}>
              <AgentsPane workspaceRoot={activeWorkspaceRoot()} />
            </Match>
            <Match when={activeActivity() === 'skills'}>
              <SkillsPane workspaceRoot={activeWorkspaceRoot()} />
            </Match>
            <Match when={activeActivity() === 'mcp'}>
              <McpPane workspaceRoot={activeWorkspaceRoot()} />
            </Match>
            <Match when={activeActivity() === 'plugins'}>
              <PluginsPane />
            </Match>
          </Switch>
        </Show>
        <main class="app-shell__content">
          {props.children}
        </main>
      </div>
      <StatusBar />
      <CommandPalette />
      <ToastHost />
    </div>
  );
};
