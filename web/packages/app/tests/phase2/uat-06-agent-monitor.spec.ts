// UAT-06 — F-137/138/139/140/152/153/156 — background agents + Agent Monitor.
// Plan: docs/testing/phase2-uat.md §UAT-06
//
// Blocked on `tauri-driver` (real `forge` background-agent lifecycle +
// MockProvider scripted spawn_background_agent + per-platform resource
// samplers). The promote-to-foreground affordance has no data-testid yet
// (instrumentation gap, plan §UAT-06) — that flow is documented as
// follow-up rather than tested here.

import { test } from '@playwright/test';

test.describe('UAT-06 — F-137/138/139/140/152/153/156 — background agents + Agent Monitor', () => {
  test.skip(
    true,
    'Blocked on tauri-driver + MockProvider bg-agent scripting + resource samplers. See docs/testing/phase2-uat.md §UAT-06.',
  );

  test('Step 1: spawn_background_agent tool call -> tool-call card shows completed; instance id returned', () => {
    // await driver.runMockToolCall('spawn_background_agent', { agent: 'worker' });
    // await expect(driver.byTestId('tool-call-card', { hasText: 'spawn_background_agent' })).toContainText('completed');
  });

  test('Step 2: navigate to /agent-monitor -> filter tablist + tabpanel render with new agent row', () => {
    // await driver.page.goto('/agent-monitor');
    // await expect(driver.locator('[role="tablist"]')).toBeVisible();
    // await expect(driver.locator('[role="tabpanel"] tr').first()).toBeVisible();
  });

  test('Step 3: row has .agent-monitor__progress[data-state="running"]', () => {
    // await expect(driver.locator('.agent-monitor__progress')).toHaveAttribute('data-state', 'running');
  });

  test('Step 4: click row -> Inspector opens with metadata + sampled CPU/memory', () => {
    // await driver.locator('[role="tabpanel"] tr').first().click();
    // await expect(driver.locator('[aria-label="Inspector"]')).toBeVisible();
    // await expect(driver.locator('[aria-label="Inspector"]')).toContainText(/cpu/i);
  });

  test('Step 5: Stop -> stop_background_agent IPC; data-state flips to done; sampler untracks', () => {
    // await driver.locator('.agent-monitor__stop').click();
    // expect(driver.ipcCalls('stop_background_agent')).toHaveLength(1);
    // await expect(driver.locator('.agent-monitor__progress')).toHaveAttribute('data-state', 'done');
    // const tracked = await driver.invoke('agent_monitor_resources');
    // expect(tracked).not.toContain(stoppedId);
  });

  test('Step 6: spawn second bg agent; switch filter to Completed -> first appears; second does not', () => {
    // await driver.runMockToolCall('spawn_background_agent', { agent: 'worker2' });
    // await driver.locator('[role="tab"]', { hasText: 'Completed' }).click();
    // await expect(driver.locator('[role="tabpanel"]', { hasText: 'worker' })).toBeVisible();
    // await expect(driver.locator('[role="tabpanel"]', { hasText: 'worker2' })).toHaveCount(0);
  });

  test('Step 7: Escape with Inspector open -> step drawer closes; otherwise no-op', () => {
    // await driver.locator('[role="tabpanel"] tr').first().click();
    // await driver.page.keyboard.press('Escape');
    // await expect(driver.locator('[role="dialog"]')).toHaveCount(0);
  });
});
