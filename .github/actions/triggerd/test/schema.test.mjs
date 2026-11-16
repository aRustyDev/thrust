import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import test from "node:test";
import Ajv2020 from "ajv/dist/2020.js";
import { parse } from "yaml";

const schema = JSON.parse(readFileSync(new URL("./fixtures/triggerd.schema.json", import.meta.url)));
const example = parse(readFileSync(new URL("./fixtures/triggerd.yml", import.meta.url), "utf8"));
const validate = new Ajv2020({ allErrors: true, strict: true }).compile(schema);
const require = createRequire(import.meta.url);
const { parsePolicy } = require("../.build/config.js");

test("adopted schema accepts the adopted example", () => assert.equal(validate(example), true, JSON.stringify(validate.errors)));
test("adopted schema rejects an unknown top-level field", () => {
  assert.equal(validate({ ...example, unknown: true }), false);
  assert.ok(validate.errors?.some((error) => error.keyword === "additionalProperties"));
});
test("adopted schema rejects an empty all expression", () => {
  const invalid = structuredClone(example);
  invalid.rules["api.inference-mock"].when = { all: [] };
  assert.equal(validate(invalid), false);
});

test("adopted schema rejects a malformed coverage disposition", () => {
  const invalid = structuredClone(example);
  invalid.coverage.docs = {
    kind: "exempt",
    owner: "documentation-maintainer",
    reason: "wrong structural shape",
    workflow: ".github/workflows/checks.yml"
  };
  assert.equal(validate(invalid), false);
  assert.ok(validate.errors?.some((error) => error.keyword === "additionalProperties"));
});

test("adopted schema rejects a non-Boolean/non-expression rule predicate", () => {
  const invalid = structuredClone(example);
  invalid.rules["api.inference-mock"].when = ["changed", "api"];
  assert.equal(validate(invalid), false);
});

test("schema structural success does not bypass semantic self-invalidation", () => {
  const semanticallyInvalid = structuredClone(example);
  semanticallyInvalid.invalidate = [".github/triggerd.yml"];
  assert.equal(validate(semanticallyInvalid), true);
  assert.throws(
    () => parsePolicy(Buffer.from(JSON.stringify(semanticallyInvalid)), ".github/triggerd.yml"),
    { name: "TriggerdError", code: "CONFIG_INVALID" }
  );
});
