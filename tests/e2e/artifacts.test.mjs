import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, mkdir, writeFile, readFile, access, rm } from "node:fs/promises";
import { join } from "node:path";
import { prepareArtifacts } from "./artifacts.mjs";
test("clears old measurements for this platform without deleting another platform result", async () => {
  const root = await mkdtemp("/tmp/forge-artifacts-");
  try {
    await mkdir(join(root, "linux"));
    await mkdir(join(root, "darwin"));
    await writeFile(join(root, "linux", "measurements.json"), "old success");
    await writeFile(join(root, "darwin", "measurements.json"), "other result");
    const dir = await prepareArtifacts("linux", { root });
    assert.equal(dir, join(root, "linux"));
    await assert.rejects(access(join(dir, "measurements.json")));
    assert.equal(await readFile(join(root, "darwin", "measurements.json"), "utf8"), "other result");
    await assert.rejects(prepareArtifacts("../outside", { root }));
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
