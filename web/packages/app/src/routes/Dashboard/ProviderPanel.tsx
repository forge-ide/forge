import { type Component, createResource, createSignal, For, Show } from 'solid-js';
import { invoke } from '../../lib/tauri';
import './ProviderPanel.css';

export interface ProviderStatus {
  reachable: boolean;
  base_url: string;
  models: string[];
  last_checked: string;
  error_kind?: string;
}

const fetchStatus = () => invoke<ProviderStatus>('provider_status');

export const ProviderPanel: Component = () => {
  const [status, { refetch }] = createResource<ProviderStatus>(fetchStatus);
  const [expanded, setExpanded] = createSignal(false);

  return (
    <section class="provider-panel" aria-label="AI provider status">
      <header class="provider-panel__header">
        <span class="provider-panel__label">PROVIDER</span>
        <span class="provider-panel__name">ollama</span>
      </header>

      <Show when={status()} fallback={<p class="provider-panel__loading">PROBING</p>}>
        {(s) => (
          <>
            <div class="provider-panel__row">
              <HealthIndicator reachable={s().reachable} />
              <code class="provider-panel__url">{s().base_url}</code>
            </div>

            <Show
              when={s().reachable}
              fallback={<UnreachableHint baseUrl={s().base_url} errorKind={s().error_kind} />}
            >
              <ModelSection
                models={s().models}
                expanded={expanded()}
                onToggle={() => setExpanded((v) => !v)}
              />
            </Show>

            <div class="provider-panel__actions">
              <button type="button" class="provider-panel__btn" onClick={() => refetch()}>
                REFRESH
              </button>
            </div>
          </>
        )}
      </Show>
    </section>
  );
};

const HealthIndicator: Component<{ reachable: boolean }> = (props) => (
  <span
    class="provider-panel__health"
    classList={{
      'provider-panel__health--ok': props.reachable,
      'provider-panel__health--down': !props.reachable,
    }}
    role="img"
    aria-label={props.reachable ? 'reachable' : 'unreachable'}
  />
);

const ModelSection: Component<{
  models: string[];
  expanded: boolean;
  onToggle: () => void;
}> = (props) => (
  <div class="provider-panel__models">
    <button
      type="button"
      class="provider-panel__btn provider-panel__btn--ghost"
      aria-expanded={props.expanded}
      onClick={props.onToggle}
    >
      {props.expanded ? 'HIDE MODELS' : 'SHOW MODELS'}
      <span class="provider-panel__count">
        {' '}
        — {props.models.length} {props.models.length === 1 ? 'model' : 'models'}
      </span>
    </button>
    <Show when={props.expanded}>
      <ul class="provider-panel__model-list">
        <For each={props.models}>
          {(m) => <li class="provider-panel__model">{m}</li>}
        </For>
      </ul>
    </Show>
  </div>
);

const UnreachableHint: Component<{ baseUrl: string; errorKind?: string | undefined }> = (props) => {
  // Voice rule: include the exact technical identifier developers expect.
  const host = props.baseUrl.replace(/^https?:\/\//, '');
  return (
    <div class="provider-panel__unreachable">
      <p class="provider-panel__error-code">ECONNREFUSED {host}</p>
      <p class="provider-panel__error-detail">
        Start the Ollama daemon. The Dashboard reprobes on refresh.
      </p>
      <a
        href="https://ollama.com/download"
        target="_blank"
        rel="noreferrer noopener"
        role="button"
        class="provider-panel__btn provider-panel__btn--primary"
      >
        START OLLAMA
      </a>
    </div>
  );
};
