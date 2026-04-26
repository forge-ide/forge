// UAT-05 — F-133/134/135/136/352 — agents + sub-agents + AGENTS.md injection.
// Plan: docs/testing/phase2-uat.md §UAT-05
//
// Blocked on `tauri-driver` (real `forge` orchestrator + MockProvider
// scripted spawn_sub_agent / agents-md injection) AND on the
// `agent-source` data-testid for provenance verification (instrumentation
// gap, plan §UAT-05). The 256 KiB cap (Step 7) requires writing a real
// AGENTS.md > the cap and observing the warning event from forged.

import { test } from '@playwright/test';

test.describe('UAT-05 — F-133/134/135/136/352 — agents + sub-agents + AGENTS.md injection', () => {
  test.skip(
    true,
    'Blocked on tauri-driver + MockProvider spawn_sub_agent scripting + agent-source selector. See docs/testing/phase2-uat.md §UAT-05.',
  );

  test('Step 1: open session with --agent orchestrator -> orchestrator shown in pane header', () => {
    // await driver.openSession({ agent: 'orchestrator', workspace: WS });
    // await expect(driver.byTestId('pane-header-subject')).toContainText('orchestrator');
  });

  test('Step 2: send "do the thing" -> spawn_sub_agent("worker", ...) tool call emitted', () => {
    // await driver.composer().fill('do the thing');
    // await driver.composer().press('Enter');
    // await expect(driver.byTestId('tool-call-card', { hasText: 'spawn_sub_agent' })).toBeVisible();
  });

  test('Step 3: ChatPane mounts sub-agent banner with header (display name + model + tool count)', () => {
    // const banner = driver.locator('[data-testid^="sub-agent-banner-"]').first();
    // await expect(banner).toBeVisible();
    // const header = banner.locator('[data-testid^="sub-agent-banner-header-"]');
    // await expect(header).toBeVisible();
  });

  test('Step 4: click banner header -> body expands with sub-agent transcript', () => {
    // await header.click();
    // const body = banner.locator('[data-testid^="sub-agent-banner-body-"]');
    // await expect(body).toContainText('worker says: hello');
  });

  test('Step 5: Tab into header, Enter -> banner toggles via keyboard (F-138 a11y)', () => {
    // await header.focus();
    // await header.press('Enter');
    // await expect(body).toBeHidden();
    // await header.press('Enter');
    // await expect(body).toBeVisible();
  });

  test('Step 6: AGENTS.md contents are injected into orchestrator first-turn system prompt', () => {
    // const sentSystemPrompt = await driver.mockProvider().lastSystemPrompt();
    // expect(sentSystemPrompt).toContain('Always greet politely.');
  });

  test('Step 7: AGENTS.md > 256 KiB -> injection truncates; warning surfaces; no crash', () => {
    // await fs.writeFile(`${WS}/AGENTS.md`, 'x'.repeat(300_000));
    // await driver.openSession({ agent: 'orchestrator', workspace: WS });
    // const sysPrompt = await driver.mockProvider().lastSystemPrompt();
    // expect(sysPrompt.length).toBeLessThanOrEqual(256 * 1024);
    // // Warning may surface as a CLI stderr line or a session event per F-352.
    // expect(driver.cliStderr()).toMatch(/AGENTS\.md.*truncat/i);
  });

  test('Step 8: double-click sub-agent banner header -> /agent-monitor?instance=<child_id>', () => {
    // await header.dblclick();
    // await expect(driver.page).toHaveURL(/\/agent-monitor\?instance=/);
  });
});
