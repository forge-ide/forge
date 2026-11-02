import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { once } from "node:events";
import { until } from "./rpc.mjs";
test("SIGTERM awaits owned process cleanup before the runner exits", async () => {
  const root = await mkdtemp("/tmp/forge-signal-");
  const signalModule = pathToFileURL(resolve("tests/e2e/signals.mjs")).href;
  const processModule = pathToFileURL(resolve("tests/e2e/process.mjs")).href;
  const worker = spawn(
    process.execPath,
    [
      "--input-type=module",
      "-e",
      `import {writeFile} from 'node:fs/promises';import {onTermination} from ${JSON.stringify(signalModule)};import {startProcess} from ${JSON.stringify(processModule)};const owned=await startProcess(process.execPath,['-e','setInterval(()=>{},1000)'],{logPath:${JSON.stringify(join(root, "child.log"))}});onTermination(async()=>{await owned.close();await writeFile(${JSON.stringify(join(root, "cleaned"))},'yes');});await writeFile(${JSON.stringify(join(root, "ready"))},String(owned.child.pid));setInterval(()=>{},1000);`,
    ],
    { stdio: ["ignore", "ignore", "pipe"] },
  );
  let childPid;
  try {
    childPid = Number(
      await until(() => readFile(join(root, "ready"), "utf8"), Boolean, { label: "signal worker" }),
    );
    const exit = once(worker, "exit");
    worker.kill("SIGTERM");
    const [code] = await exit;
    assert.equal(code, 143);
    assert.equal(await readFile(join(root, "cleaned"), "utf8"), "yes");
    assert.throws(() => process.kill(childPid, 0));
  } finally {
    worker.kill("SIGKILL");
    if (childPid)
      try {
        process.kill(childPid, "SIGKILL");
      } catch {
        /* The owned child may already have exited. */
      }
    await rm(root, { recursive: true, force: true });
  }
});
