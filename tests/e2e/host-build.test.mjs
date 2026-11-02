import { test } from "node:test";
import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import { join } from "node:path";
async function scripts(root) {
  return (
    await Promise.all(
      (await readdir(join(root, "assets")))
        .filter((file) => file.endsWith(".js"))
        .map((file) => readFile(join(root, "assets", file), "utf8")),
    )
  ).join("\n");
}
test("normal desktop build has no network-test transport and browser build has no Tauri IPC host", async () => {
  const desktop = await scripts("packages/ui/dist"),
    web = await scripts("target/e2e-web");
  assert(desktop.includes("daemon_connect"));
  assert(!desktop.includes("Test bridge connection closed"));
  assert(web.includes("Test bridge connection closed"));
  assert(!web.includes("daemon_connect"));
});
