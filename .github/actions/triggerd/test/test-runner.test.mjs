import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { validateSummary } from "../scripts/test-summary.mjs";

test("accepts a successful non-empty complete summary", () => {
  assert.equal(validateSummary({ type: "test:summary", success: true, tests: 1, failed: 0, cancelled: 0, skipped: 0, todo: 0 }), undefined);
});

test("normalizes the final Node 24 summary rather than a per-file wrapper", () => {
  const event = {
    type: "test:summary",
    data: { success: true, counts: { tests: 7, failed: 0, passed: 7, cancelled: 0, skipped: 0, todo: 0 } }
  };
  const normalized = {
    type: event.type,
    success: event.data.success,
    tests: event.data.counts.tests,
    failed: event.data.counts.failed,
    cancelled: event.data.counts.cancelled,
    skipped: event.data.counts.skipped,
    todo: event.data.counts.todo
  };
  assert.equal(validateSummary(normalized), undefined);
});

for (const [name, summary] of [
  ["assertion failure", { type: "test:summary", success: false, tests: 1, failed: 1, cancelled: 0, skipped: 0, todo: 0 }],
  ["empty suite", { type: "test:summary", success: true, tests: 0, failed: 0, cancelled: 0, skipped: 0, todo: 0 }],
  ["skip", { type: "test:summary", success: true, tests: 1, failed: 0, cancelled: 0, skipped: 1, todo: 0 }],
  ["todo", { type: "test:summary", success: true, tests: 1, failed: 0, cancelled: 0, skipped: 0, todo: 1 }],
  ["cancellation", { type: "test:summary", success: false, tests: 1, failed: 0, cancelled: 1, skipped: 0, todo: 0 }],
  ["missing summary", undefined]
]) test(`rejects ${name}`, () => assert.ok(validateSummary(summary)));

for (const fixture of ["fail", "empty", "skip", "todo", "cancelled"]) test(`runner rejects an actual ${fixture} fixture`, () => {
  const root = fileURLToPath(new URL("..", import.meta.url));
  const result = spawnSync(process.execPath, ["scripts/test-runner.mjs", `test/fixtures/runner/${fixture}.test.mjs`], { cwd: root, encoding: "utf8" });
  assert.equal(result.status, 1, result.stderr);
});
