import assert from "node:assert/strict";
import { createRequire } from "node:module";
import test from "node:test";

const require = createRequire(import.meta.url);
const { serializeEvaluation } = require("../.build/report.js");

const evaluation = {
  decisions: {
    "z.check": { selected: false, reason: "not-matched", groups: { z: false } },
    "a.check": { selected: true, reason: "matched", groups: { b: false, a: true } }
  },
  changeSet: { kind: "complete", count: 0 },
  coverageWarnings: ["z", "a"]
};
const context = {
  scope: "",
  event: "pull_request",
  activity: "synchronize",
  source: {
    repository: "aRustyDev/thrust",
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

test("serializes compact deterministic rules and details from exact policy bytes", () => {
  const first = serializeEvaluation(evaluation, Buffer.from("version: 1\n# retained\n"), context);
  const reordered = serializeEvaluation({ ...evaluation, decisions: { "a.check": evaluation.decisions["a.check"], "z.check": evaluation.decisions["z.check"] }, coverageWarnings: ["a", "z"] }, Buffer.from("version: 1\n# retained\n"), context);
  assert.equal(first.rules, '{"a.check":true,"z.check":false}');
  assert.equal(first.details, reordered.details);
  assert.match(first.details, /"configSha256":"/);
  assert.match(first.summary, /a\.check: execute/);
  assert.doesNotMatch(first.summary, /<|&|::/);
});

test("raw policy bytes, complete zero, and not-collected remain distinct", () => {
  const first = serializeEvaluation(evaluation, Buffer.from("version: 1\n"), context);
  const changedBytes = serializeEvaluation(evaluation, Buffer.from("version: 1\n# comment\n"), context);
  assert.notEqual(first.details, changedBytes.details);
  const full = serializeEvaluation({ ...evaluation, changeSet: { kind: "not-collected", reason: "manual" } }, Buffer.from("version: 1\n"), context);
  assert.match(first.details, /"count":0/);
  assert.match(full.details, /"not-collected"/);
});

test("rejects output and summary size overages without exposing unsafe content", () => {
  assert.throws(() => serializeEvaluation({ ...evaluation, decisions: Object.fromEntries(Array.from({ length: 10000 }, (_, index) => [`a${index}.check`, evaluation.decisions["a.check"]])) }, Buffer.from("version: 1"), context), { code: "OUTPUT_INVALID" });
  assert.throws(() => serializeEvaluation(evaluation, Buffer.from("version: 1"), { ...context, activity: "<script>" }), { code: "OUTPUT_INVALID" });
});
