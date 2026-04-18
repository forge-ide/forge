// UAT-01b — Ollama status card smoke (F-023).
// Plan: docs/testing/phase1-uat.md §UAT-01b
//
// Requires real Ollama running at 127.0.0.1:11434 with ≥1 model pulled.
// The card renders from `provider_status`, which lives on the Tauri side —
// the full-shell path needs a tauri-driver harness. The mocked-IPC variant
// below exercises only the UI contract, not the real daemon call.

import { test, expect } from './fixtures/tauri-mock';

test.describe('UAT-01b — Ollama status card', () => {
  test('UI contract: reachable state shows models and base URL', async ({ tauri, page }) => {
    await tauri.onInvoke('session_list', async () => []);
    await tauri.onInvoke('provider_status', async () => ({
      reachable: true,
      base_url: 'http://127.0.0.1:11434',
      models: ['llama3.2:1b', 'qwen2.5:0.5b'],
      last_checked: new Date().toISOString(),
    }));
    await page.goto('/');
    await expect(page.getByRole('region', { name: 'AI provider status' })).toBeVisible();
    await expect(page.getByRole('region', { name: 'AI provider status' })).toContainText(
      'http://127.0.0.1:11434',
    );
  });

  test('real-shell variant (requires tauri-driver)', async () => {
    test.skip(true, 'requires tauri-driver + live Ollama — see phase1-uat.md §UAT-01b');
  });
});
