import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { runEvaluationWorkerMeasured } = require("../.build/worker.js");
const { serializeEvaluation } = require("../.build/report.js");

const componentCount = 100;
const pathCount = 5000;
const warmups = 2;
const iterations = 10;
const policy = {
  invalidate: [".github/triggerd.yml", ".github/actions/triggerd/**"],
  changes: Object.fromEntries(Array.from({ length: componentCount }, (_, index) => [`c${index}`, { paths: [`components/c${index}/**`] }])),
  rules: Object.fromEntries(Array.from({ length: componentCount }, (_, index) => [`bench.c${index}`, { description: `benchmark component ${index}`, events: ["pull_request"], when: { changed: `c${index}` } }]))
};
const input = {
  policy,
  context: { event: "pull_request", scope: "", fullCheck: false },
  changes: Array.from({ length: pathCount }, (_, index) => ({ status: "modified", path: `components/c${index % componentCount}/src/f${index}.py` }))
};
const policyBytes = Buffer.from(JSON.stringify(policy));
const reportContext = {
  scope: "",
  event: "pull_request",
  activity: "synchronize",
  source: {
    repository: "benchmark/triggerd",
    adapter: "pr-api",
    before: null,
    base: "1".repeat(40),
    head: "2".repeat(40),
    mergeBase: "1".repeat(40),
    policyRevision: "3".repeat(40),
    actionRevision: "3".repeat(40),
    testRevision: "3".repeat(40)
  }
};

async function sample() {
  const started = performance.now();
  const result = await runEvaluationWorkerMeasured(input, 10000);
  const elapsedMs = performance.now() - started;
  assert.equal(result.evaluation.changeSet.count, pathCount);
  assert.equal(Object.keys(result.evaluation.decisions).length, componentCount);
  assert.equal(Object.values(result.evaluation.decisions).filter((decision) => decision.selected).length, componentCount);
  return { elapsedMs, workerRss: result.workerRss, evaluation: result.evaluation };
}

for (let index = 0; index < warmups; index += 1) await sample();
const samples = [];
for (let index = 0; index < iterations; index += 1) samples.push(await sample());
const times = samples.map((sample) => sample.elapsedMs).sort((left, right) => left - right);
const quantile = (fraction) => times[Math.min(times.length - 1, Math.ceil(times.length * fraction) - 1)];
const report = serializeEvaluation(samples.at(-1).evaluation, policyBytes, reportContext);
assert.equal(Object.keys(JSON.parse(report.rules)).length, componentCount);
assert.equal(Object.keys(JSON.parse(report.details).decisions).length, componentCount);
const machineOutputBytes = Buffer.byteLength(report.rules) + Buffer.byteLength(report.details);
const summaryBytes = Buffer.byteLength(report.summary);
process.stdout.write(`${JSON.stringify({
  evidence: "local-only",
  node: process.version,
  platform: `${process.platform}-${process.arch}`,
  generator: "5000 modified components/c(i mod 100)/src/f(i).py; 100 groups/rules; two required invalidations",
  warmups,
  iterations,
  wallMs: { median: quantile(0.5), p95: quantile(0.95), max: times.at(-1) },
  workerRssBytes: Math.max(...samples.map((sample) => sample.workerRss)),
  machineOutputBytes,
  summaryBytes,
  changeSet: { kind: "complete", count: pathCount },
  selectedRules: componentCount
})}\n`);
