import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import test from "node:test";

const require = createRequire(import.meta.url);
const { parsePolicy } = require("../.build/config.js");
const { LIMITS } = require("../.build/limits.js");
const example = readFileSync(new URL("./fixtures/triggerd.yml", import.meta.url));

test("parses the adopted valid policy", () => {
  const policy = parsePolicy(example, ".github/triggerd.yml");
  assert.equal(policy.version, 1);
  assert.equal(policy.rules["api.inference-mock"].when.any.length, 4);
  assert.equal(Object.getPrototypeOf(policy), null);
  assert.equal(Object.getPrototypeOf(policy.changes), null);
  assert.equal(Object.getPrototypeOf(policy.rules), null);
});

const validPolicy = (overrides = "") => `version: 1
invalidate: [.github/triggerd.yml, .github/actions/triggerd/**]
changes: {api: {paths: [apps/api/**]}, docs: {paths: [docs/**]}}
rules: {api.check: {description: API check, events: [pull_request], when: {changed: api}}}
${overrides}`;
const rejects = (source, name = "CONFIG_INVALID") => assert.throws(
  () => parsePolicy(Buffer.from(source), ".github/triggerd.yml"),
  { name: "TriggerdError", code: name }
);

test("requires both the installed policy and triggerd implementation invalidations", () => {
  rejects(validPolicy().replace(", .github/actions/triggerd/**", ""));
});

for (const [name, source] of [
  ["quoted version", validPolicy().replace("version: 1", "version: '1'")],
  ["unknown policy field", validPolicy("unexpected: true")],
  ["unknown rule field", validPolicy().replace("when: {changed: api}", "when: {changed: api}, extra: true")],
  ["empty all", validPolicy().replace("{changed: api}", "{all: []}")],
  ["empty any", validPolicy().replace("{changed: api}", "{any: []}")],
  ["multiple expression operators", validPolicy().replace("{changed: api}", "{changed: api, not: true}")],
  ["unknown expression operator", validPolicy().replace("{changed: api}", "{unknown: true}")],
  ["unsupported activity", validPolicy().replace("{changed: api}", "{activity: synchronize}")],
  ["quoted Boolean expression", validPolicy().replace("{changed: api}", "'true'")],
  ["unknown event", validPolicy().replace("pull_request", "pull_request_target")],
  ["duplicate event", validPolicy().replace("[pull_request]", "[pull_request, pull_request]")],
  ["invalid group identifier", validPolicy().replace("api: {paths", "api_name: {paths")],
  ["invalid rule identifier", validPolicy().replace("api.check", "api.check.more")],
  ["non-array invalidation", validPolicy().replace("[.github/triggerd.yml, .github/actions/triggerd/**]", ".github/triggerd.yml")],
  ["non-map changes", validPolicy().replace("changes: {api: {paths: [apps/api/**]}, docs: {paths: [docs/**]}}", "changes: []")],
  ["non-array group exclusion", validPolicy().replace("paths: [apps/api/**]", "paths: [apps/api/**], exclude: docs/**")],
  ["non-map rules", validPolicy().replace("rules: {api.check: {description: API check, events: [pull_request], when: {changed: api}}}", "rules: []")],
  ["non-string rule description", validPolicy().replace("description: API check", "description: false")],
  ["non-array events", validPolicy().replace("events: [pull_request]", "events: pull_request")],
  ["coverage for consumed group", validPolicy("coverage: {api: {kind: exempt, owner: ci, reason: owned}}")],
  ["coverage for unknown group", validPolicy("coverage: {missing: {kind: exempt, owner: ci, reason: owned}}")],
  ["exempt disposition with external fields", validPolicy("coverage: {docs: {kind: exempt, owner: ci, reason: owned, workflow: .github/workflows/ci.yml}}")],
  ["external disposition with nonflat workflow", validPolicy("coverage: {docs: {kind: external, owner: ci, reason: owned, workflow: .github/workflows/nested/ci.yml, job: docs}}")],
  ["external disposition with non-string job", validPolicy("coverage: {docs: {kind: external, owner: ci, reason: owned, workflow: .github/workflows/ci.yml, job: 1}}")],
  ["unknown disposition kind", validPolicy("coverage: {docs: {kind: ignore, owner: ci, reason: owned}}")],
  ["reserved key nested in a rule", validPolicy().replace("{changed: api}", "{not: {constructor: true}}")]
]) test(`rejects ${name}`, () => rejects(source));

test("rejects invalid UTF-8 without exposing policy content", () => {
  assert.throws(
    () => parsePolicy(new Uint8Array([0xff]), ".github/triggerd.yml"),
    (error) => error?.code === "CONFIG_INVALID" && error.message === "Policy validation failed"
  );
});

test("enforces expression depth and counts every expression node", () => {
  let deep = "true";
  for (let index = 0; index < 16; index += 1) deep = `{not: ${deep}}`;
  rejects(validPolicy().replace("{changed: api}", deep));

  const withinLimit = `{any: [${Array(8191).fill("true").join(", ")}]}`;
  assert.equal(parsePolicy(Buffer.from(validPolicy().replace("{changed: api}", withinLimit)), ".github/triggerd.yml").version, 1);
  const overLimit = `{any: [${Array(8192).fill("true").join(", ")}]}`;
  rejects(validPolicy().replace("{changed: api}", overLimit));
});

test("enforces policy byte limits exactly", () => {
  const base = validPolicy();
  const fill = LIMITS.policyBytes - Buffer.byteLength(base) - 2;
  const atLimit = `${base}\n#${"x".repeat(fill)}`;
  assert.equal(Buffer.byteLength(atLimit), LIMITS.policyBytes);
  assert.equal(parsePolicy(Buffer.from(atLimit), ".github/triggerd.yml").version, 1);
  rejects(`${atLimit}x`);
});

test("enforces group, rule, and total-pattern limits exactly", () => {
  const groups = (count) => Array.from({ length: count }, (_, index) => `g${index}: {paths: [src/g${index}/**]}`).join("\n  ");
  const rules = (count) => Array.from({ length: count }, (_, index) => `r${index}.check: {description: check ${index}, events: [pull_request], when: true}`).join("\n  ");
  const policyWith = (changes, ruleSet) => `version: 1
invalidate: [.github/triggerd.yml, .github/actions/triggerd/**]
changes:
  ${changes}
rules:
  ${ruleSet}`;
  assert.equal(parsePolicy(Buffer.from(policyWith(groups(LIMITS.groups), rules(1))), ".github/triggerd.yml").version, 1);
  rejects(policyWith(groups(LIMITS.groups + 1), rules(1)));
  assert.equal(parsePolicy(Buffer.from(policyWith("api: {paths: [apps/api/**]}", rules(LIMITS.rules))), ".github/triggerd.yml").version, 1);
  rejects(policyWith("api: {paths: [apps/api/**]}", rules(LIMITS.rules + 1)));

  const patterns = (count) => Array.from({ length: count }, (_, index) => `src/p${index}/**`).join(", ");
  const policyWithPatterns = (count) => validPolicy().replace("apps/api/**", patterns(count));
  assert.equal(parsePolicy(Buffer.from(policyWithPatterns(LIMITS.patterns - 3)), ".github/triggerd.yml").version, 1);
  rejects(policyWithPatterns(LIMITS.patterns - 2));
});

for (const [name, source] of [
  ["duplicate keys", "version: 1\nversion: 1\ninvalidate: [.github/triggerd.yml]\nchanges: {a: {paths: [a]}}\nrules: {a.b: {description: x, events: [pull_request], when: true}}"],
  ["anchor", "version: 1\ninvalidate: &x [.github/triggerd.yml]\nchanges: {a: {paths: [a]}}\nrules: {a.b: {description: x, events: [pull_request], when: true}}"],
  ["alias", "version: 1\ninvalidate: [.github/triggerd.yml]\nchanges: {a: &x {paths: [a]}, b: *x}\nrules: {a.b: {description: x, events: [pull_request], when: true}}"],
  ["tag", "version: !!int 1\ninvalidate: [.github/triggerd.yml]\nchanges: {a: {paths: [a]}}\nrules: {a.b: {description: x, events: [pull_request], when: true}}"],
  ["merge", "version: 1\ninvalidate: [.github/triggerd.yml]\nchanges: {a: {paths: [a]}}\nrules: {<<: {a.b: {description: x, events: [pull_request], when: true}}}"],
  ["multiple documents", "version: 1\ninvalidate: [.github/triggerd.yml]\nchanges: {a: {paths: [a]}}\nrules: {a.b: {description: x, events: [pull_request], when: true}}\n---\n{}"],
  ["reserved key", "version: 1\ninvalidate: [.github/triggerd.yml]\nchanges: {__proto__: {paths: [a]}}\nrules: {a.b: {description: x, events: [pull_request], when: true}}"],
  ["unknown rule group", "version: 1\ninvalidate: [.github/triggerd.yml]\nchanges: {a: {paths: [a]}}\nrules: {a.b: {description: x, events: [pull_request], when: {changed: missing}}}"],
  ["missing self invalidation", "version: 1\ninvalidate: [other.yml]\nchanges: {a: {paths: [a]}}\nrules: {a.b: {description: x, events: [pull_request], when: true}}"],
  ["empty group path", "version: 1\ninvalidate: [.github/triggerd.yml]\nchanges: {a: {paths: ['']}}\nrules: {a.b: {description: x, events: [pull_request], when: true}}"],
  ["unsupported group glob", "version: 1\ninvalidate: [.github/triggerd.yml]\nchanges: {a: {paths: ['a/{b,c}']}}\nrules: {a.b: {description: x, events: [pull_request], when: true}}"],
  ["external disposition without job", "version: 1\ninvalidate: [.github/triggerd.yml]\nchanges: {a: {paths: [a]}}\ncoverage: {a: {kind: external, owner: x, reason: x, workflow: .github/workflows/checks.yml, job: ''}}\nrules: {a.b: {description: x, events: [pull_request], when: true}}"]
]) test(`rejects ${name}`, () => assert.throws(() => parsePolicy(Buffer.from(source), ".github/triggerd.yml"), { name: "TriggerdError", code: "CONFIG_INVALID" }));
