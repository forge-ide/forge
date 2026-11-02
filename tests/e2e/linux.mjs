import { remote } from "webdriverio";
import { execFileSync } from "node:child_process";
import { createServer } from "node:net";
import { once } from "node:events";
import { writeFile } from "node:fs/promises";
import { resolve, join } from "node:path";
import { startProcess } from "./process.mjs";
import { until } from "./rpc.mjs";
async function unusedPort() {
  const server = createServer();
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const port = server.address().port;
  await new Promise((resolve) => server.close(resolve));
  return port;
}
export async function startBrowser(fixture, artifactDir) {
  const port = await unusedPort(),
    nativePort = await unusedPort();
  const driverPath = resolve("target/e2e-tools/bin/tauri-driver");
  const driverArgs = ["--port", String(port), "--native-port", String(nativePort)];
  const headed = !!fixture.env.DISPLAY;
  const webkitVersion = execFileSync("pkg-config", ["--modversion", "webkit2gtk-4.1"], {
    encoding: "utf8",
  }).trim();
  const launch = {
    webkitVersion,
    display: headed ? "existing" : "Xvfb",
    startedAt: new Date().toISOString(),
    connected: false,
    elapsedMs: 0,
  };
  const launchStarted = performance.now();
  await writeFile(join(artifactDir, "launch.json"), JSON.stringify(launch, null, 2));
  const driver = await startProcess(
    headed ? driverPath : "xvfb-run",
    headed
      ? driverArgs
      : ["-a", "--server-args=-screen 0 1280x800x24 -nolisten tcp", driverPath, ...driverArgs],
    { env: fixture.env, logPath: join(artifactDir, "tauri-driver.log") },
  );
  let browser;
  try {
    await until(
      async () => {
        driver.assertRunning();
        return await fetch(`http://127.0.0.1:${port}/status`);
      },
      (response) => response.ok,
      { label: "Tauri WebDriver" },
    );
    browser = await remote({
      hostname: "127.0.0.1",
      port,
      logLevel: "error",
      connectionRetryCount: 0,
      connectionRetryTimeout: 60000,
      capabilities: { "tauri:options": { application: resolve("target/debug/forge-desktop") } },
    });
    await writeFile(
      join(artifactDir, "launch.json"),
      JSON.stringify(
        { ...launch, connected: true, elapsedMs: performance.now() - launchStarted },
        null,
        2,
      ),
    );
    await browser.setTimeout({ script: 20000, implicit: 0, pageLoad: 20000 });
    return {
      platform: "Linux real Tauri / WebKitGTK",
      version: `WebKitGTK ${webkitVersion} / Wry ${browser.capabilities.browserVersion ?? "unknown"}`,
      evaluate: (fn, arg) => browser.execute(fn, arg),
      click: async (selector) => {
        const el = await browser.$(selector);
        await el.waitForDisplayed({ timeout: 15000 });
        await el.click();
      },
      fill: async (selector, value) => {
        const el = await browser.$(selector);
        await el.waitForDisplayed({ timeout: 15000 });
        await el.setValue(value);
      },
      screenshot: (path) => browser.saveScreenshot(path),
      async close() {
        try {
          await browser.deleteSession();
        } finally {
          await driver.close();
        }
      },
    };
  } catch (error) {
    await writeFile(
      join(artifactDir, "launch.json"),
      JSON.stringify({ ...launch, elapsedMs: performance.now() - launchStarted }, null, 2),
    ).catch(() => {});
    if (browser) await browser.deleteSession().catch(() => {});
    await driver.close();
    throw error;
  }
}
