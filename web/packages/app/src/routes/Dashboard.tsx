import { type Component, createResource, Show } from 'solid-js';
import { ProviderPanel } from './Dashboard/ProviderPanel';
import { SessionsPanel } from './Dashboard/SessionsPanel';
import {
  CREDENTIAL_PROVIDERS,
  CredentialBanner,
  CredentialsSection,
} from '../components/dashboard/CredentialsSection';
import { hasCredential } from '../ipc/credentials';
import './Dashboard.css';

/**
 * F-588: surface a first-run banner when the active provider has no stored
 * credential. Phase 3 ships two credential-bearing providers
 * (`anthropic`, `openai`); without an explicit "active provider" signal
 * yet, the banner picks the first such provider that lacks a credential
 * so the user has a concrete target to add.
 *
 * Probe failure on a single provider is treated as "stored for that
 * provider only" — the loop `continue`s so a broken probe on Anthropic
 * does not silently suppress the banner that would otherwise call out a
 * missing OpenAI key. The per-row indicator inside `<CredentialsSection>`
 * surfaces the underlying probe error to the user.
 */
async function firstMissingCredential(): Promise<{ id: string; label: string } | null> {
  for (const provider of CREDENTIAL_PROVIDERS) {
    try {
      const stored = await hasCredential(provider.id);
      if (!stored) return { id: provider.id as unknown as string, label: provider.label };
    } catch {
      continue;
    }
  }
  return null;
}

export const Dashboard: Component = () => {
  const [missing] = createResource(firstMissingCredential);

  return (
    <main class="dashboard">
      <h1 class="dashboard__title">Forge — Dashboard</h1>
      <Show when={missing()}>
        {(m) => <CredentialBanner providerLabel={m().label} />}
      </Show>
      <ProviderPanel />
      <CredentialsSection />
      <SessionsPanel />
    </main>
  );
};
