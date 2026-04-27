// F-588: per-provider credential management UI on the Dashboard.
//
// Renders one row per provider that supports credentials. Each row shows a
// presence indicator (✓ when stored, ⚠ when missing) backed by the F-587
// `has_credential` Tauri command, an inline form to enter or replace the
// key, and a logout button when a credential is present.
//
// Rotation contract: when a credential already exists, submitting a new
// value pops a confirmation modal first. Rotation is destructive (the
// previous key is overwritten in the keyring with no recovery), so the
// UX gates it behind an explicit confirm step. Logout is reversible
// (re-enter the key) and stays single-step.
//
// Security contract:
//   - The key value lives only in a single `<input type="password">`'s
//     local state and is cleared the moment the IPC call resolves.
//   - The DOM never contains a rendered key — only the `password` input
//     (browser-DOM-only) ever holds the typed value.
//   - Logging / aria-labels never echo the key.

import {
  type Component,
  createResource,
  createSignal,
  For,
  Show,
} from 'solid-js';
import { Button } from '@forge/design';
import type { ProviderId } from '@forge/ipc';
import { hasCredential, loginProvider, logoutProvider } from '../../ipc/credentials';
import { useFocusTrap } from '../../lib/useFocusTrap';
import './CredentialsSection.css';

// ---------------------------------------------------------------------------
// Provider catalogue
// ---------------------------------------------------------------------------

/**
 * Phase 3 ships two providers that require credentials. Ollama is keyless
 * and intentionally absent from this list — adding it would surface a
 * "missing key" indicator for a provider that does not need one.
 *
 * The mapping is duplicated from `forge-core::credentials::env` (the
 * `EnvFallbackStore::default` mapping). When a third provider lands the
 * change is two lines (this list + the env mapping) — the rest of the
 * section is provider-agnostic.
 */
export interface CredentialProvider {
  id: ProviderId;
  /** Human-readable name shown next to the indicator. */
  label: string;
  /** Env var name, surfaced as a hint when no credential is stored. */
  envHint: string;
}

export const CREDENTIAL_PROVIDERS: CredentialProvider[] = [
  { id: 'anthropic' as ProviderId, label: 'Anthropic', envHint: 'ANTHROPIC_API_KEY' },
  { id: 'openai' as ProviderId, label: 'OpenAI', envHint: 'OPENAI_API_KEY' },
];

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export const CredentialsSection: Component = () => {
  return (
    <section class="credentials-section" aria-label="Provider credentials">
      <header class="credentials-section__header">
        <span class="credentials-section__label">CREDENTIALS</span>
      </header>

      <ul class="credentials-section__list" role="list">
        <For each={CREDENTIAL_PROVIDERS}>
          {(provider) => <CredentialRow provider={provider} />}
        </For>
      </ul>
    </section>
  );
};

// ---------------------------------------------------------------------------
// Row — one per provider
// ---------------------------------------------------------------------------

interface CredentialRowProps {
  provider: CredentialProvider;
}

const CredentialRow: Component<CredentialRowProps> = (props) => {
  const [present, { refetch }] = createResource(
    () => props.provider.id,
    (id) => hasCredential(id),
  );
  const [draft, setDraft] = createSignal('');
  const [pending, setPending] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [pendingRotation, setPendingRotation] = createSignal<string | null>(null);

  const submit = async (value: string) => {
    setPending(true);
    setError(null);
    try {
      await loginProvider(props.provider.id, value);
      // Per security contract, drop the value from local state the moment
      // the IPC call resolves so the runtime heap no longer holds a copy.
      setDraft('');
      await refetch();
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setPending(false);
      setPendingRotation(null);
    }
  };

  const onSubmit = (e: SubmitEvent) => {
    e.preventDefault();
    const value = draft();
    if (value.length === 0 || pending()) return;

    if (present()) {
      // Existing credential → rotation confirm modal.
      setPendingRotation(value);
      return;
    }
    // First-time storage → no confirmation needed.
    void submit(value);
  };

  const onLogout = async () => {
    setPending(true);
    setError(null);
    try {
      await logoutProvider(props.provider.id);
      await refetch();
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setPending(false);
    }
  };

  const indicatorLabel = () =>
    present() ? `Credential stored for ${props.provider.label}` : `No credential for ${props.provider.label}`;

  const inputId = `credential-input-${props.provider.id}`;

  return (
    <li class="credentials-section__row" data-testid={`credential-row-${props.provider.id}`}>
      <div class="credentials-section__row-head">
        <Indicator stored={present() === true} label={indicatorLabel()} />
        <span class="credentials-section__provider-label">{props.provider.label}</span>
        <span class="credentials-section__env-hint" aria-hidden="true">
          {props.provider.envHint}
        </span>
        <Show when={present()}>
          <Button
            variant="ghost"
            size="sm"
            class="credentials-section__btn"
            data-testid={`credential-logout-${props.provider.id}`}
            aria-label={`Remove stored credential for ${props.provider.label}`}
            aria-busy={pending()}
            disabled={pending()}
            onClick={() => void onLogout()}
          >
            LOGOUT
          </Button>
        </Show>
      </div>

      <form class="credentials-section__form" onSubmit={onSubmit}>
        <label class="credentials-section__field-label" for={inputId}>
          {present() ? 'Replace key' : 'Add key'}
        </label>
        <input
          id={inputId}
          class="credentials-section__input"
          data-testid={`credential-input-${props.provider.id}`}
          type="password"
          autocomplete="off"
          spellcheck={false}
          value={draft()}
          onInput={(e) => setDraft(e.currentTarget.value)}
          aria-label={`API key for ${props.provider.label}`}
          disabled={pending()}
        />
        <Button
          variant="primary"
          size="sm"
          class="credentials-section__btn credentials-section__btn--primary"
          data-testid={`credential-submit-${props.provider.id}`}
          aria-busy={pending()}
          disabled={pending() || draft().length === 0}
          type="submit"
        >
          {present() ? 'ROTATE' : 'STORE'}
        </Button>
      </form>

      <Show when={error()}>
        <p class="credentials-section__error" role="alert" data-testid={`credential-error-${props.provider.id}`}>
          {error()}
        </p>
      </Show>

      <Show when={pendingRotation() !== null}>
        <RotationConfirm
          providerLabel={props.provider.label}
          onCancel={() => setPendingRotation(null)}
          onConfirm={() => {
            const value = pendingRotation();
            if (value !== null) void submit(value);
          }}
          pending={pending()}
        />
      </Show>
    </li>
  );
};

// ---------------------------------------------------------------------------
// Indicator (✓ / ⚠) — never reveals the value
// ---------------------------------------------------------------------------

const Indicator: Component<{ stored: boolean; label: string }> = (props) => (
  <span
    class="credentials-section__indicator"
    classList={{
      'credentials-section__indicator--stored': props.stored,
      'credentials-section__indicator--missing': !props.stored,
    }}
    role="img"
    aria-label={props.label}
  >
    {props.stored ? '✓' : '⚠'}
  </span>
);

// ---------------------------------------------------------------------------
// Rotation confirmation modal
// ---------------------------------------------------------------------------

interface RotationConfirmProps {
  providerLabel: string;
  pending: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}

const RotationConfirm: Component<RotationConfirmProps> = (props) => {
  let dialogRef: HTMLDivElement | undefined;
  useFocusTrap(() => dialogRef);

  const onKey = (e: KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      props.onCancel();
    }
  };

  return (
    <div
      class="credentials-section__modal-backdrop"
      onClick={(e) => {
        if (e.target === e.currentTarget) props.onCancel();
      }}
      data-testid="credential-rotation-modal"
    >
      <div
        ref={dialogRef}
        class="credentials-section__modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="credential-rotation-title"
        onKeyDown={onKey}
      >
        <header class="credentials-section__modal-head">
          <h3 id="credential-rotation-title" class="credentials-section__modal-title">
            REPLACE STORED KEY?
          </h3>
        </header>
        <p class="credentials-section__modal-body">
          A credential for <strong>{props.providerLabel}</strong> is already stored. Replacing it
          overwrites the keyring entry — the previous key cannot be recovered.
        </p>
        <div class="credentials-section__modal-actions">
          <Button
            variant="ghost"
            size="sm"
            class="credentials-section__btn"
            data-testid="credential-rotation-cancel"
            disabled={props.pending}
            onClick={props.onCancel}
          >
            CANCEL
          </Button>
          <Button
            variant="primary"
            size="sm"
            class="credentials-section__btn credentials-section__btn--primary"
            data-testid="credential-rotation-confirm"
            aria-busy={props.pending}
            disabled={props.pending}
            onClick={props.onConfirm}
          >
            REPLACE
          </Button>
        </div>
      </div>
    </div>
  );
};

// ---------------------------------------------------------------------------
// First-run banner — surfaced from Dashboard.tsx, not the section above.
// ---------------------------------------------------------------------------

interface CredentialBannerProps {
  /** Display name of the provider missing a credential. */
  providerLabel: string;
}

/**
 * First-run prompt on the Dashboard when the active provider has no stored
 * credential. Anchored above the Credentials section it points to.
 */
export const CredentialBanner: Component<CredentialBannerProps> = (props) => (
  <div
    class="credentials-banner"
    role="status"
    data-testid="credential-banner"
    aria-label={`${props.providerLabel} has no stored credential`}
  >
    <span class="credentials-banner__icon" aria-hidden="true">
      ⚠
    </span>
    <p class="credentials-banner__text">
      <strong>{props.providerLabel}</strong> has no stored credential. Add one in the Credentials
      section below to start a session.
    </p>
  </div>
);
