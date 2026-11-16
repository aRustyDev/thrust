import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import test from "node:test";
import { parse } from "yaml";

const require = createRequire(import.meta.url);
const { normalizeChanges } = require("../.build/changes.js");
const { runEvaluationWorker } = require("../.build/worker.js");
const policy = parse(readFileSync(new URL("./fixtures/triggerd.yml", import.meta.url), "utf8"));
const fixture = JSON.parse(readFileSync(new URL("./fixtures/decisions.json", import.meta.url)));

for (const item of fixture.cases) test(`decision fixture: ${item.name}`, async () => {
  const merged = { ...fixture.defaults, ...item.context };
  const fullCheck = item.fullCheck ?? item.changes === null;
  const context = {
    event: merged.event,
    scope: merged.scope,
    fullCheck,
    ...(fullCheck ? { notCollectedReason: item.fullCheck ? "explicit-full-check" : merged.event === "workflow_dispatch" ? "manual" : merged.event === "schedule" ? "schedule" : "new-branch" } : {})
  };
  const changes = context.fullCheck ? null : normalizeChanges(item.changes);
  const evaluation = await runEvaluationWorker({ policy, context, changes }, 1000);
  assert.deepEqual(Object.fromEntries(Object.entries(evaluation.decisions).map(([id, decision]) => [id, decision.selected])), item.expected);
  assert.deepEqual(evaluation.changeSet, item.expectedChangeSet ?? { kind: "complete", count: changes.length });
});
