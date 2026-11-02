import { webkit } from "playwright";
import { resolve } from "node:path";
import { startBridge } from "./bridge.mjs";
export async function startBrowser(fixture) {
  const bridge = await startBridge({
    socketPath: resolve(fixture.env.FORGE_RUNTIME_DIR, "forged.sock"),
    assetRoot: resolve("target/e2e-web"),
  });
  let browser;
  try {
    browser = await webkit.launch({ headless: false, env: fixture.env });
    const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
    page.setDefaultTimeout(15000);
    await page.goto(bridge.url);
    return {
      platform: "macOS Playwright WebKit proxy (not native Tauri)",
      version: browser.version(),
      evaluate: (fn, arg) => page.evaluate(fn, arg),
      click: (selector) => page.locator(selector).click(),
      fill: (selector, value) => page.locator(selector).fill(value),
      screenshot: (path) => page.screenshot({ path, fullPage: true }),
      async close() {
        try {
          await browser.close();
        } finally {
          await bridge.close();
        }
      },
    };
  } catch (error) {
    await browser?.close();
    await bridge.close();
    throw error;
  }
}
