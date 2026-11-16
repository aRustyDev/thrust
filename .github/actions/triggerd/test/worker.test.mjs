import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { parse } from "yaml";

const require = createRequire(import.meta.url);
const { runEvaluationWorker } = require("../.build/worker.js");
const policy = parse(readFileSync(new URL("./fixtures/triggerd.yml", import.meta.url), "utf8"));
const fixtureWorker = (name) => fileURLToPath(new URL(`fixtures/${name}`, import.meta.url));
const input = { policy, context: { event: "pull_request", scope: "api", fullCheck: false }, changes: [{ status: "modified", path: "apps/api/a.py" }] };

test("evaluates in the compiled isolated worker", async () => {
  const evaluation = await runEvaluationWorker(input, 1000);
  assert.deepEqual(
    Object.fromEntries(Object.entries(evaluation.decisions).map(([id, decision]) => [id, decision.selected])),
    { "api.inference-mock": true }
  );
});

test("parent deadline rejects before a worker can publish a result", async () => {
  await assert.rejects(
    runEvaluationWorker(input, 0),
    { code: "DEADLINE_EXCEEDED" }
  );
});

test("parent timer remains responsive and terminates a busy worker with no result", async () => {
  let parentTimerFired = false;
  const parentTimer = setTimeout(() => { parentTimerFired = true; }, 5);
  await assert.rejects(runEvaluationWorker(input, 30, { workerPath: fixtureWorker("worker-busy.cjs") }), { code: "DEADLINE_EXCEEDED" });
  clearTimeout(parentTimer);
  assert.equal(parentTimerFired, true);
});

test("worker errors and early exits fail promptly without waiting for the deadline", async () => {
  const started = performance.now();
  await assert.rejects(runEvaluationWorker(input, 1000, { workerPath: fixtureWorker("worker-error.cjs") }), { code: "INPUT_INVALID" });
  await assert.rejects(runEvaluationWorker(input, 1000, { workerPath: fixtureWorker("worker-exit.cjs") }), { code: "INPUT_INVALID" });
  assert.ok(performance.now() - started < 500);
});

test("remaining total budget takes precedence over the worker budget", async () => {
  await assert.rejects(
    runEvaluationWorker(input, 1000, { remainingMs: 1, workerPath: fixtureWorker("worker-busy.cjs") }),
    { code: "DEADLINE_EXCEEDED" }
  );
});

test("termination cleans up busy workers before a later healthy evaluation", async () => {
  for (let index = 0; index < 3; index += 1) {
    await assert.rejects(runEvaluationWorker(input, 10, { workerPath: fixtureWorker("worker-busy.cjs") }), { code: "DEADLINE_EXCEEDED" });
  }
  const evaluation = await runEvaluationWorker(input, 1000);
  assert.equal(evaluation.decisions["api.inference-mock"].selected, true);
});

test("adversarial allowed glob structure remains bounded in the isolated worker", async () => {
  const adversarial = {
    invalidate: [".github/triggerd.yml", ".github/actions/triggerd/**"],
    changes: { api: { paths: [`${Array(8).fill("**").join("/")}/never`] } },
    rules: { "api.check": { description: "api", events: ["pull_request"], when: { changed: "api" } } }
  };
  const path = Array.from({ length: 64 }, () => "segment").join("/");
  const result = await runEvaluationWorker({ policy: adversarial, context: { event: "pull_request", scope: "", fullCheck: false }, changes: [{ status: "modified", path }] }, 1000);
  assert.deepEqual(
    { selected: result.decisions["api.check"].selected, reason: result.decisions["api.check"].reason },
    { selected: true, reason: "unmatched-path" }
  );
});
