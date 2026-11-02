import assert from "node:assert/strict";
import { join } from "node:path";
import { until } from "./rpc.mjs";
import { assertReviewFlow, openingProbe, scrollingProbe } from "./measure.mjs";
const button = (text) => `//button[normalize-space(.)='${text}']`;
async function waitText(browser, text) {
  return await until(
    () => browser.evaluate((text) => document.body.textContent.includes(text), text),
    Boolean,
    { label: `visible ${text}` },
  );
}
async function openSession(browser, title) {
  await until(
    () =>
      browser.evaluate(
        (title) =>
          [...document.querySelectorAll(".session-row")].some(
            (el) => el.querySelector("span")?.textContent === title,
          ),
        title,
      ),
    Boolean,
    { label: `session ${title}` },
  );
  await browser.click(`//button[contains(@class,'session-row')][span[1]='${title}']`);
  await until(() => browser.evaluate(() => !!document.querySelector(".file-header")), Boolean, {
    label: "selected file",
  });
}
export async function runReviewFlow(browser, fixture, artifactDir) {
  await openSession(browser, fixture.loop.title);
  await until(() => browser.evaluate(() => !!document.querySelector(".cm-line")), Boolean, {
    label: "initial diff",
  });
  await browser.fill('input[aria-label="Start line"]', "2");
  await browser.fill('input[aria-label="End line"]', "2");
  await browser.click(button("Select lines"));
  await browser.fill(".comment-composer textarea", "Read the retry limit from configuration");
  await browser.click(button("Save comment"));
  await until(
    () => fixture.rpc.call("review.threads", { sessionId: fixture.loop.id }),
    (threads) =>
      threads.length === 1 &&
      threads[0].comments[0].body === "Read the retry limit from configuration",
    { label: "saved daemon comment" },
  );
  await waitText(browser, "Request changes (1)");
  await browser.click(button("Request changes (1)"));
  await waitText(browser, "Moved to RetryConfig::max_attempts.");
  await browser.click(button("Resolve"));
  const threads = await until(
    () => fixture.rpc.call("review.threads", { sessionId: fixture.loop.id }),
    (threads) => threads.length === 1 && threads[0].status === "resolved",
    { label: "persisted resolved thread" },
  );
  const changeset = await fixture.rpc.call("review.changeset", { sessionId: fixture.loop.id });
  assertReviewFlow(threads[0], changeset.round);
  await waitText(browser, "Show resolved thread");
  await browser.screenshot(join(artifactDir, "review-loop.png"));
  await browser.click(button("Back to projects"));
  const openings = [],
    scrolls = [];
  for (let index = 0; index < 5; index++) {
    await until(
      () =>
        browser.evaluate(
          (title) =>
            [...document.querySelectorAll(".session-row")].some(
              (el) => el.querySelector("span")?.textContent === title,
            ),
          fixture.opening.title,
        ),
      Boolean,
      { label: "performance session" },
    );
    openings.push(await browser.evaluate(openingProbe, { title: fixture.opening.title }));
    await browser.click(button("Back to projects"));
  }
  await openSession(browser, fixture.opening.title);
  for (let file = 0; file < 5; file++) {
    await browser.click(`//button[contains(@class,'file-path')][contains(.,'file-${file}.rs')]`);
    await until(() => browser.evaluate(() => !!document.querySelector(".cm-line")), Boolean, {
      label: "file code",
    });
    scrolls.push(await browser.evaluate(scrollingProbe, { lastLine: `FINAL ${file} 1000` }));
  }
  await browser.screenshot(join(artifactDir, "performance.png"));
  await browser.click(button("Back to projects"));
  await openSession(browser, fixture.large.title);
  await waitText(browser, "Diff starts collapsed");
  assert.equal(
    await browser.evaluate(() => document.querySelectorAll(".cm-line").length),
    0,
    "Large file must not eagerly render",
  );
  await browser.click(button("Load diff"));
  await until(() => browser.evaluate(() => !!document.querySelector(".cm-line")), Boolean, {
    label: "loaded 5,000-line diff",
  });
  scrolls.push(await browser.evaluate(scrollingProbe, { lastLine: "FINAL 0 5000" }));
  return {
    platform: browser.platform,
    browserVersion: browser.version,
    openings,
    scrolls,
    flow: { threadId: threads[0].id, status: threads[0].status, round: changeset.round },
  };
}
