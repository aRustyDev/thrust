import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import test from "node:test";

test("performance profile measures worker output from the production report serializer", () => {
  const result = spawnSync(process.execPath, ["scripts/performance.mjs"], { encoding: "utf8" });
  assert.equal(result.status, 0, result.stderr);
  const profile = JSON.parse(result.stdout);
  assert.equal(profile.machineOutputBytes > 0, true);
  assert.equal(profile.summaryBytes > 0, true);
  assert.equal(profile.machineOutputBytes >= profile.summaryBytes, true);
});
