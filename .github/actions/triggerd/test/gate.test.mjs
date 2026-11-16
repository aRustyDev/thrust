import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import test from "node:test";
import { parse } from "yaml";

const require = createRequire(import.meta.url);
const { verifyGate, validateBindings } = require("../.build/gate.js");

const bindings = {
  planner: "plan",
  bindings: { "api.check": "api", "ui.check": "ui" },
  needs: { api: ["plan"], ui: ["plan", "api"] }
};
const needs = { plan: { result: "success" }, api: { result: "success" }, ui: { result: "skipped" } };
const fixturePolicy = parse(readFileSync(new URL("./fixtures/triggerd.yml", import.meta.url), "utf8"));
const fixtureBindings = JSON.parse(readFileSync(new URL("./fixtures/bindings.json", import.meta.url), "utf8"));

test("accepts exact Boolean decisions with successful selected work", () => {
  assert.deepEqual(verifyGate({ "api.check": true, "ui.check": false }, needs, bindings), { selected: ["api.check"], overhead: [] });
});

test("static bindings require exact IDs, unique consumers, and planner closure", () => {
  assert.doesNotThrow(() => validateBindings(["api.check", "ui.check"], bindings));
  assert.throws(() => validateBindings(["api.check", "ui.check"], { ...bindings, bindings: { "api.check": "api" } }), { code: "OUTPUT_INVALID" });
  assert.throws(() => validateBindings(["api.check", "ui.check"], { ...bindings, bindings: { "api.check": "api", "ui.check": "api" } }), { code: "OUTPUT_INVALID" });
  assert.throws(() => validateBindings(["api.check", "ui.check"], { ...bindings, needs: { ...bindings.needs, ui: ["api"] } }), { code: "OUTPUT_INVALID" });
});

test("binding removal mutation fails independent static binding validation", () => {
  const removedBinding = structuredClone(fixtureBindings);
  delete removedBinding.bindings["ui.contract-tests"];
  assert.throws(() => validateBindings(Object.keys(fixturePolicy.rules), removedBinding), { code: "OUTPUT_INVALID" });
});

test("gate CLI consumes only fixed JSON environment inputs", () => {
  const result = spawnSync(process.execPath, [new URL("../.build/gate-main.js", import.meta.url).pathname], {
    encoding: "utf8",
    env: { ...process.env, RULES_JSON: JSON.stringify({ "api.check": true, "ui.check": false }), NEEDS_JSON: JSON.stringify(needs), BINDINGS_JSON: JSON.stringify(bindings) }
  });
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(JSON.parse(result.stdout), { selected: ["api.check"], overhead: [] });
});

for (const [name, rules, result, code] of [
  ["planner failure", { "api.check": true, "ui.check": false }, { ...needs, plan: { result: "failure" } }, "OUTPUT_INVALID"],
  ["selected skipped", { "api.check": true, "ui.check": false }, { ...needs, api: { result: "skipped" } }, "OUTPUT_INVALID"],
  ["unselected failure", { "api.check": false, "ui.check": false }, { ...needs, api: { result: "failure" } }, "OUTPUT_INVALID"],
  ["unknown result", { "api.check": true, "ui.check": false }, { ...needs, api: { result: "neutral" } }, "OUTPUT_INVALID"],
  ["missing output key", { "api.check": true }, needs, "OUTPUT_INVALID"],
  ["extra output key", { "api.check": true, "ui.check": false, extra: false }, needs, "OUTPUT_INVALID"],
  ["non Boolean output", { "api.check": "true", "ui.check": false }, needs, "OUTPUT_INVALID"],
  ["missing prerequisite", { "api.check": true, "ui.check": false }, needs, { ...bindings, needs: { ...bindings.needs, api: [] } }]
]) test(`rejects ${name}`, () => assert.throws(() => verifyGate(rules, result, code === undefined ? bindings : code), { code: "OUTPUT_INVALID" }));
