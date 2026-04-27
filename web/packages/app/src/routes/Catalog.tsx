// F-592: Dashboard-mounted Catalog route.
//
// Hosts the `<CatalogPane>` component and pulls its required `workspaceRoot`
// out of the URL query string (`?ws=<workspace_root>`). The route lives on
// the Dashboard window so the pane's `list_*` calls clear the F-591
// `dashboard` window-label gate; the workspace string is validated against
// the registry by the shell-side handler before any disk I/O.
//
// When `?ws=` is missing we render a friendly "open from a session" notice
// instead of fabricating a path — the F-591 contract rejects unregistered
// workspace roots, so silently passing an empty string would only surface
// as a confusing IPC error.

import { type Component, Show } from 'solid-js';
import { useSearchParams } from '@solidjs/router';
import { CatalogPane } from '../components/catalog/CatalogPane';
import './Catalog.css';

export const Catalog: Component = () => {
  const [searchParams] = useSearchParams<{ ws?: string }>();
  const workspaceRoot = (): string | null => {
    const raw = searchParams.ws;
    if (!raw) return null;
    return raw.trim().length === 0 ? null : raw;
  };

  return (
    <main class="catalog-route">
      <Show
        when={workspaceRoot()}
        fallback={
          <section class="catalog-route__missing" aria-label="Catalog requires a workspace">
            <h1 class="catalog-route__title">Catalog</h1>
            <p class="catalog-route__hint">
              Open the catalog from a session window or pass a registered
              workspace path via the <code>?ws=</code> query parameter.
            </p>
          </section>
        }
      >
        {(ws) => <CatalogPane workspaceRoot={ws()} />}
      </Show>
    </main>
  );
};
