import { writeFile } from "node:fs/promises";
import { join } from "node:path";
import { recordPerformance } from "./report.mjs";
import { prepareArtifacts } from "./artifacts.mjs";
import { startFixture } from "./fixture.mjs";
import { runReviewFlow } from "./review-flow.mjs";
import { onTermination } from "./signals.mjs";
const artifactDir = await prepareArtifacts(process.platform);
const controller = new AbortController();
let fixtureStart, browserStart, closing, browser;
const cleanup = () =>
  (closing ??= (async () => {
    const ownedBrowser = await browserStart?.catch(() => undefined);
    try {
      await ownedBrowser?.close();
    } finally {
      const fixture = await fixtureStart?.catch(() => undefined);
      await fixture?.close();
    }
  })());
const off = onTermination(cleanup, { abort: () => controller.abort() });
try {
  fixtureStart = startFixture({ artifactDir, signal: controller.signal });
  const fixture = await fixtureStart;
  controller.signal.throwIfAborted();
  const adapter = await import(process.platform === "linux" ? "./linux.mjs" : "./macos.mjs");
  controller.signal.throwIfAborted();
  browserStart = adapter.startBrowser(fixture, artifactDir);
  browser = await browserStart;
  controller.signal.throwIfAborted();
  await writeFile(
    join(artifactDir, "environment.json"),
    JSON.stringify({ platform: browser.platform, browserVersion: browser.version }, null, 2),
  );
  const result = await recordPerformance(
    await runReviewFlow(browser, fixture, artifactDir),
    artifactDir,
  );
  console.log(
    JSON.stringify({
      ...result,
      scrolls: result.scrolls.map(({ frames, ...scroll }) => ({
        ...scroll,
        frameCount: frames.length,
        maximumFrame: Math.max(...frames),
      })),
    }),
  );
} catch (error) {
  if (!controller.signal.aborted) {
    await browser?.screenshot(join(artifactDir, "failure.png")).catch(() => {});
    await writeFile(join(artifactDir, "failure.txt"), String(error.stack ?? error));
    console.error(error);
    process.exitCode = 1;
  }
} finally {
  try {
    await cleanup();
  } finally {
    off();
  }
}
