import { mkdir, rm } from "node:fs/promises";
import { resolve, join } from "node:path";
/** Each invocation owns its platform output; a failure cannot leave an old success report. */
export async function prepareArtifacts(platform, { root = resolve("target/e2e-artifacts") } = {}) {
  if (!["linux", "darwin"].includes(platform))
    throw new Error("Unsupported test artifact platform");
  const directory = join(root, platform);
  await rm(directory, { recursive: true, force: true });
  await mkdir(directory, { recursive: true });
  return directory;
}
