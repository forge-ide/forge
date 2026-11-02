import { writeFile } from "node:fs/promises";
import { join } from "node:path";
import { assertPerformance } from "./measure.mjs";
/** Save the evidence before asserting so a failure cannot erase its raw samples. */
export async function recordPerformance(result, directory) {
  await writeFile(
    join(directory, "measurements.json"),
    JSON.stringify({ ...result, passed: false }, null, 2),
  );
  const summary = assertPerformance(result.openings, result.scrolls);
  const verified = { ...result, ...summary, passed: true };
  await writeFile(join(directory, "measurements.json"), JSON.stringify(verified, null, 2));
  return verified;
}
