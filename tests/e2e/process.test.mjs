import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { join } from "node:path";
import { startProcess } from "./process.mjs";
import { until } from "./rpc.mjs";
for (const leaderExits of [false, true])
  test(`cleanup stops a SIGTERM-resistant descendant when leader ${leaderExits ? "already exited" : "exits during shutdown"}`, async () => {
    const root = await mkdtemp("/tmp/forge-process-");
    const pidFile = join(root, "child.pid");
    const descendant = `require('node:fs').writeFileSync(${JSON.stringify(pidFile)},String(process.pid));process.on('SIGTERM',()=>{});setInterval(()=>{},1000);`;
    const parent = `require('node:child_process').spawn(process.execPath,['-e',${JSON.stringify(descendant)}],{stdio:'ignore'});${leaderExits ? "setTimeout(()=>process.exit(0),100);" : "setInterval(()=>{},1000);"}`;
    const owned = await startProcess(process.execPath, ["-e", parent], {
      logPath: join(root, "parent.log"),
    });
    try {
      const pid = Number(
        await until(() => readFile(pidFile, "utf8"), Boolean, { label: "resistant descendant" }),
      );
      if (leaderExits)
        await until(
          () => owned.child.exitCode,
          (code) => code === 0,
          { label: "parent exit" },
        );
      await owned.close();
      assert.throws(() => process.kill(pid, 0));
    } finally {
      try {
        process.kill(-owned.child.pid, "SIGKILL");
      } catch {
        /* The owned group may already have exited. */
      }
      await owned.close();
      await rm(root, { recursive: true, force: true });
    }
  });
