import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import test from "node:test";
import { parse } from "yaml";

const require = createRequire(import.meta.url);
const { parseOracle, verifyCoverage, verifyMutations } = require("../.build/coverage.js");
const policy = parse(readFileSync(new URL("./fixtures/triggerd.yml", import.meta.url), "utf8"));
const oracle = JSON.parse(readFileSync(new URL("./fixtures/coverage-oracle.json", import.meta.url)));

test("independent reviewed oracle accepts the policy and every named mutation fails", () => {
  const parsed = parseOracle(oracle, policy);
  assert.deepEqual(verifyCoverage(policy, parsed).overhead["foo alone is insufficient for interaction"], []);
  assert.doesNotThrow(() => verifyMutations(policy, parsed));
});

test("oracle rejects unknown references and overlapping expectations", () => {
  assert.throws(() => parseOracle({ ...oracle, cases: [{ ...oracle.cases[0], required: ["missing.rule"], forbidden: [] }] }, policy), { code: "INPUT_INVALID" });
  assert.throws(() => parseOracle({ ...oracle, cases: [{ ...oracle.cases[0], required: ["security.secrets"], forbidden: ["security.secrets"] }] }, policy), { code: "INPUT_INVALID" });
});
