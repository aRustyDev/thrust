import assert from "node:assert/strict";
import { createRequire } from "node:module";
import test from "node:test";

const require = createRequire(import.meta.url);
const { parseRuntimeContext } = require("../.build/context.js");
const sha = (digit) => digit.repeat(40);
const payload = {
  action: "synchronize",
  repository: { full_name: "aRustyDev/thrust" },
  pull_request: { number: 7, changed_files: 1, base: { sha: sha("1") }, head: { sha: sha("2"), repo: { full_name: "fork/example" } } }
};

test("preserves PR head independently from a synthetic checkout revision", () => {
  const context = parseRuntimeContext("pull_request", payload, sha("3"));
  assert.deepEqual(context.source, {
    repository: "aRustyDev/thrust", adapter: "pr-api", before: null,
    base: sha("1"), head: sha("2"), mergeBase: sha("1"),
    policyRevision: sha("3"), actionRevision: sha("3"), testRevision: sha("3")
  });
  assert.equal(context.activity, "synchronize");
  assert.equal(context.prNumber, 7);
});

test("keeps title-edit PR context on the pinned API adapter", () => {
  const context = parseRuntimeContext("pull_request", { ...payload, action: "edited" }, sha("3"));
  assert.equal(context.activity, "edited");
  assert.equal(context.source.adapter, "pr-api");
  assert.equal(context.source.head, sha("2"));
});

test("manual and schedule contexts are full mode without PR identities", () => {
  const manual = parseRuntimeContext("workflow_dispatch", { repository: { full_name: "aRustyDev/thrust" } }, sha("4"));
  assert.equal(manual.fullCheck, true);
  assert.equal(manual.notCollectedReason, "manual");
  assert.equal(manual.prNumber, null);
  const schedule = parseRuntimeContext("schedule", { repository: { full_name: "aRustyDev/thrust" } }, sha("4"));
  assert.equal(schedule.notCollectedReason, "schedule");
});

test("uses exact push before and after identities, with zero-before as new-branch full mode", () => {
  const push = parseRuntimeContext("push", {
    ref: "refs/heads/main", before: sha("4"), after: sha("5"), deleted: false,
    repository: { full_name: "aRustyDev/thrust" }
  }, sha("5"));
  assert.equal(push.fullCheck, false);
  assert.deepEqual(push.source, {
    repository: "aRustyDev/thrust", adapter: "git", before: sha("4"),
    base: sha("4"), head: sha("5"), mergeBase: null,
    policyRevision: sha("5"), actionRevision: sha("5"), testRevision: sha("5")
  });
  const newBranch = parseRuntimeContext("push", {
    ref: "refs/heads/new", before: "0".repeat(40), after: sha("6"), deleted: false,
    repository: { full_name: "aRustyDev/thrust" }
  }, sha("6"));
  assert.equal(newBranch.fullCheck, true);
  assert.equal(newBranch.notCollectedReason, "new-branch");
});

for (const [name, event, input] of [
  ["missing PR identity", "pull_request", { action: "opened" }],
  ["bad SHA", "pull_request", { ...payload, pull_request: { ...payload.pull_request, head: { ...payload.pull_request.head, sha: "abc" } } }],
  ["deleted push", "push", { before: sha("1"), after: sha("2"), deleted: true, repository: { full_name: "aRustyDev/thrust" } }],
  ["stale push", "push", { ref: "refs/heads/main", before: sha("1"), after: sha("2"), deleted: false, repository: { full_name: "aRustyDev/thrust" } }],
  ["unsupported event", "pull_request_target", payload]
]) test(`rejects ${name}`, () => assert.throws(() => parseRuntimeContext(event, input, sha("3")), { code: name === "stale push" ? "SOURCE_STALE" : "INPUT_INVALID" }));
