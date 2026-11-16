import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import test from "node:test";
import { parse } from "yaml";

const require = createRequire(import.meta.url);
const { normalizeChanges, classifyChanges } = require("../.build/changes.js");
const { LIMITS } = require("../.build/limits.js");
const policy = parse(readFileSync(new URL("./fixtures/triggerd.yml", import.meta.url), "utf8"));

test("rename preserves both old and new paths for group truths", () => {
  const changes = normalizeChanges([{ status: "renamed", path: "src/bar/a.py", previousPath: "src/foo/a.py" }]);
  const result = classifyChanges(policy, changes);
  assert.equal(result.groups["foo-python"], true);
  assert.equal(result.groups["bar-python"], true);
  assert.deepEqual(result.unmatched, []);
});

test("unused broad group does not account for a new service", () => {
  const result = classifyChanges(policy, normalizeChanges([{ status: "added", path: "src/new-service/main.py" }]));
  assert.equal(result.groups["other-source"], true);
  assert.deepEqual(result.unmatched, ["src/new-service/main.py"]);
});

test("narrow external coverage accounts only matching paths", () => {
  const result = classifyChanges(policy, normalizeChanges([{ status: "modified", path: "docs/src/example.md" }]));
  assert.deepEqual(result.unmatched, []);
});

test("narrow exempt coverage accounts only matching paths and reports unowned groups", () => {
  const result = classifyChanges(policy, normalizeChanges([{ status: "modified", path: "src/foo/fixture.txt" }]));
  assert.deepEqual(result.unmatched, []);
  assert.deepEqual(result.coverageWarnings, ["other-source"]);
  assert.equal(Object.getPrototypeOf(result.groups), null);
});

test("an exclusion from the only accounted group restores unmatched fallback", () => {
  const withExclusion = structuredClone(policy);
  withExclusion.changes.docs.exclude = ["docs/private/**"];
  const result = classifyChanges(withExclusion, normalizeChanges([{ status: "modified", path: "docs/private/plan.md" }]));
  assert.equal(result.groups.docs, false);
  assert.deepEqual(result.unmatched, ["docs/private/plan.md"]);
});

test("overlapping referenced groups both account for their matching path", () => {
  const overlapping = structuredClone(policy);
  overlapping.changes["api-copy"] = { paths: ["apps/api/**"] };
  overlapping.rules["api.copy"] = { description: "copy", events: ["pull_request"], when: { changed: "api-copy" } };
  const result = classifyChanges(overlapping, normalizeChanges([{ status: "modified", path: "apps/api/a.py" }]));
  assert.equal(result.groups.api, true);
  assert.equal(result.groups["api-copy"], true);
  assert.deepEqual(result.unmatched, []);
});

test("delete plus add conservatively matches the equivalent rename", () => {
  const renamed = classifyChanges(policy, normalizeChanges([{ status: "renamed", path: "src/bar/a.py", previousPath: "src/foo/a.py" }]));
  const deleteAdd = classifyChanges(policy, normalizeChanges([
    { status: "removed", path: "src/foo/a.py" },
    { status: "added", path: "src/bar/a.py" }
  ]));
  assert.deepEqual(deleteAdd.groups, renamed.groups);
  assert.deepEqual(deleteAdd.unmatched, renamed.unmatched);
});

test("copies include their supplied source and preserve all valid statuses", () => {
  const copied = classifyChanges(policy, normalizeChanges([{ status: "copied", path: "src/bar/a.py", previousPath: "src/foo/a.py" }]));
  assert.equal(copied.groups["foo-python"], true);
  assert.equal(copied.groups["bar-python"], true);

  for (const status of ["added", "modified", "removed", "type-changed"]) {
    assert.deepEqual(normalizeChanges([{ status, path: "src/foo/a.py" }]), [{ status, path: "src/foo/a.py" }]);
  }
  assert.deepEqual(normalizeChanges([{ status: "copied", path: "src/foo/a.py" }]), [{ status: "copied", path: "src/foo/a.py" }]);
});

for (const [name, records] of [
  ["missing rename source", [{ status: "renamed", path: "a" }]],
  ["same rename source and destination", [{ status: "renamed", path: "a", previousPath: "a" }]],
  ["conflicting rename source", [{ status: "renamed", path: "b", previousPath: "a" }, { status: "renamed", path: "c", previousPath: "a" }]],
  ["previous path on an add", [{ status: "added", path: "a", previousPath: "b" }]],
  ["unknown status", [{ status: "unknown", path: "a" }]],
  ["duplicate record", [{ status: "added", path: "a" }, { status: "added", path: "a" }]],
  ["NUL path", [{ status: "added", path: "a\0b" }]],
  ["invalid UTF-8 surrogate", [{ status: "added", path: "\ud800" }]],
  ["backslash path", [{ status: "added", path: "a\\b" }]],
  ["absolute path", [{ status: "added", path: "/a" }]],
  ["traversal path", [{ status: "added", path: "a/../b" }]],
  ["too many path bytes", [{ status: "added", path: "a".repeat(LIMITS.pathBytes + 1) }]],
  ["too many path segments", [{ status: "added", path: Array.from({ length: LIMITS.pathSegments + 1 }, () => "a").join("/") }]],
  ["non-record", [null]]
]) test(`rejects ${name}`, () => assert.throws(() => normalizeChanges(records), { code: "INPUT_INVALID" }));

test("counts raw records before deduplication and preflights matcher work", () => {
  const maximumRecords = Array.from({ length: LIMITS.rawChanges }, (_, index) => ({ status: "added", path: `a${index}` }));
  assert.equal(normalizeChanges(maximumRecords).length, LIMITS.rawChanges);
  const duplicateRecords = Array.from({ length: LIMITS.rawChanges + 1 }, () => ({ status: "added", path: "a" }));
  assert.throws(() => normalizeChanges(duplicateRecords), { code: "LIMIT_EXCEEDED" });

  const patternHeavyPolicy = {
    invalidate: [".github/triggerd.yml", ".github/actions/triggerd/**"],
    changes: { api: { paths: Array.from({ length: LIMITS.patterns - 2 }, (_, index) => `src/p${index}/**`) } },
    rules: { "api.check": { when: { changed: "api" } } }
  };
  const records = Array.from({ length: Math.floor(LIMITS.comparisons / LIMITS.patterns) + 1 }, (_, index) => ({ status: "added", path: `src/change-${index}` }));
  assert.throws(() => classifyChanges(patternHeavyPolicy, normalizeChanges(records)), { code: "LIMIT_EXCEEDED" });
});

test("accepts paths at exact byte and segment limits", () => {
  assert.equal(normalizeChanges([{ status: "added", path: "a".repeat(LIMITS.pathBytes) }]).length, 1);
  assert.equal(normalizeChanges([{ status: "added", path: Array.from({ length: LIMITS.pathSegments }, () => "a").join("/") }]).length, 1);
});
