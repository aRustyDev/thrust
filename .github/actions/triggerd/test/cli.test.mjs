import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

const packageRoot = new URL("../", import.meta.url);
const policy = new URL("./fixtures/triggerd.yml", import.meta.url).pathname;
const context = new URL("fixtures/cli-context.json", import.meta.url).pathname;
const changes = new URL("fixtures/cli-changes.json", import.meta.url).pathname;
const fullChanges = new URL("fixtures/cli-full-changes.json", import.meta.url).pathname;
const malformed = new URL("fixtures/cli-malformed.txt", import.meta.url).pathname;
const run = (...args) => spawnSync(process.execPath, [new URL(".build/main.js", packageRoot).pathname, ...args], { encoding: "utf8" });

test("validate accepts the exact required rules flag", () => {
  const result = run("validate", "--rules", policy);
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.stdout, '{"valid":true}\n');
});

test("evaluate emits a single rules/details object through production evaluation", () => {
  const result = run("evaluate", "--rules", policy, "--context", context, "--changes", changes, "--scope", "api", "--full-check", "false");
  assert.equal(result.status, 0, result.stderr);
  const output = JSON.parse(result.stdout);
  assert.deepEqual(output.rules, { "api.inference-mock": true });
  assert.equal(output.details.changeSet.kind, "complete");
});

// A policy with one rule that selects on the pull-request activity alone, written here so the test does not depend on a
// repository's own .github/triggerd.yml. The same policy with an unedited context is the control: the rule must be false there.
const activityPolicy = `version: 1
invalidate: [.github/triggerd.yml, .github/actions/triggerd/**]
changes:
  docs: {paths: [docs/**]}
rules:
  docs.checks:
    description: Run the docs checks when the pull request is edited
    events: [pull_request]
    when: {any: [{changed: docs}, {activity: edited}]}
`;

test("evaluate passes activity predicates into the production worker", () => {
  const directory = mkdtempSync(join(tmpdir(), "triggerd-cli-"));
  const rules = join(directory, "triggerd.yml");
  const editedContext = join(directory, "context.json");
  const noChanges = join(directory, "changes.json");
  try {
    writeFileSync(rules, activityPolicy);
    writeFileSync(editedContext, readFileSync(context, "utf8").replace("synchronize", "edited"));
    writeFileSync(noChanges, "[]");
    const edited = run("evaluate", "--rules", rules, "--context", editedContext, "--changes", noChanges, "--full-check", "false");
    assert.equal(edited.status, 0, edited.stderr);
    assert.equal(JSON.parse(edited.stdout).rules["docs.checks"], true);
    const synchronized = run("evaluate", "--rules", rules, "--context", context, "--changes", noChanges, "--full-check", "false");
    assert.equal(synchronized.status, 0, synchronized.stderr);
    assert.equal(JSON.parse(synchronized.stdout).rules["docs.checks"], false);
  } finally { rmSync(directory, { recursive: true, force: true }); }
});

test("full mode requires null records and preserves not-collected details", () => {
  const result = run("evaluate", "--rules", policy, "--context", context, "--changes", fullChanges, "--full-check", "true");
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(JSON.parse(result.stdout).details.changeSet, { kind: "not-collected", reason: "explicit-full-check" });
  const rejected = run("evaluate", "--rules", policy, "--context", context, "--changes", changes, "--full-check", "true");
  assert.equal(rejected.status, 1);
});

test("source and JSON failures use safe validation exits", () => {
  const missing = run("validate", "--rules", "/private/tmp/triggerd-not-a-policy-secret.yml");
  assert.equal(missing.status, 1);
  assert.doesNotMatch(missing.stderr, /secret/);
  const broken = run("evaluate", "--rules", policy, "--context", malformed, "--changes", changes);
  assert.equal(broken.status, 1);
  assert.equal(broken.stdout, "");
});

for (const args of [[], ["validate", "--rules", policy, "--rules", policy], ["validate", "--unknown"], ["evaluate", "--rules", policy, "--context", context, "--changes", changes, "--full-check", "maybe"]]) test(`rejects CLI syntax ${args.join(" ")}`, () => {
  const result = run(...args);
  assert.equal(result.status, 2);
  assert.equal(result.stdout, "");
  assert.match(result.stderr, /triggerd:/);
});
