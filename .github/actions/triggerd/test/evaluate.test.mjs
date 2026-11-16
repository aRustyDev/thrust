import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import test from "node:test";
import { parse } from "yaml";

const require = createRequire(import.meta.url);
const { evaluatePolicy } = require("../.build/evaluate.js");
const policy = parse(readFileSync(new URL("./fixtures/triggerd.yml", import.meta.url), "utf8"));
const context = { event: "pull_request", scope: "", fullCheck: false };
const evaluate = (changes, override = {}) => evaluatePolicy(policy, { ...context, ...override }, changes);
const decision = (changes, id, override) => evaluate(changes, override).decisions[id];
const plainDecision = (value) => ({ ...value, groups: { ...value.groups } });

test("selects foo interaction only when both groups occur in the net diff", () => {
  assert.equal(decision([{ status: "modified", path: "src/foo/a.py" }], "foo.both-python").selected, false);
  assert.equal(decision([{ status: "modified", path: "src/bar/a.py" }], "foo.both-python").selected, false);
  assert.equal(decision([], "foo.both-python").selected, false);
  assert.deepEqual(plainDecision(decision([{ status: "modified", path: "src/foo/a.py" }, { status: "modified", path: "src/bar/a.py" }], "foo.both-python")), {
    selected: true,
    reason: "matched",
    groups: { "bar-python": true, "foo-python": true }
  });
});

test("evaluates nested any/all/not and preserves referenced false group values", () => {
  const nested = structuredClone(policy);
  nested.rules["nested.check"] = {
    description: "nested",
    events: ["pull_request"],
    when: { all: [{ any: [{ changed: "foo-python" }, { changed: "bar-python" }] }, { not: { changed: "api" } }] }
  };
  const selected = evaluatePolicy(nested, context, [{ status: "modified", path: "src/foo/a.py" }]).decisions["nested.check"];
  assert.equal(Object.getPrototypeOf(selected.groups), null);
  assert.deepEqual(plainDecision(selected), { selected: true, reason: "matched", groups: { api: false, "bar-python": false, "foo-python": true } });
  const rejected = evaluatePolicy(nested, context, [{ status: "modified", path: "src/foo/a.py" }, { status: "modified", path: "apps/api/a.py" }]).decisions["nested.check"];
  assert.equal(rejected.selected, false);
  assert.equal(rejected.reason, "not-matched");
});

test("literal false remains selectable under conservative full mode", () => {
  const withFalse = structuredClone(policy);
  withFalse.rules["false.check"] = { description: "false", events: ["pull_request"], when: false };
  assert.deepEqual(plainDecision(evaluatePolicy(withFalse, context, []).decisions["false.check"]), { selected: false, reason: "not-matched", groups: {} });
  assert.deepEqual(plainDecision(evaluatePolicy(withFalse, { ...context, fullCheck: true }, null).decisions["false.check"]), { selected: true, reason: "full-check", groups: {} });
});

test("activity predicates select title edits without treating an unchanged diff as full mode", () => {
  const activity = structuredClone(policy);
  activity.rules["title.check"] = { description: "title", events: ["pull_request"], when: { activity: "edited" } };
  const unchanged = evaluatePolicy(activity, { event: "pull_request", activity: "edited", scope: "", fullCheck: false }, []);
  assert.equal(unchanged.decisions["title.check"].selected, true);
  assert.equal(unchanged.decisions["foo.both-python"].selected, false);
  const synchronize = evaluatePolicy(activity, { event: "pull_request", activity: "synchronize", scope: "", fullCheck: false }, []);
  assert.equal(synchronize.decisions["title.check"].selected, false);
});

test("event eligibility precedes full selection and unsupported events fail", () => {
  assert.deepEqual(plainDecision(evaluate(null, { event: "push", fullCheck: true }).decisions["docs.api-review"]), { selected: false, reason: "event-ineligible", groups: {} });
  assert.throws(() => evaluate(null, { event: "pull_request_target", fullCheck: true }), { code: "INPUT_INVALID" });
});

test("projects exact scope keys and rejects unknown scopes and collection-mode confusion", () => {
  assert.deepEqual(Object.keys(evaluate([], { scope: "api" }).decisions), ["api.inference-mock"]);
  assert.throws(() => evaluate([], { scope: "missing" }), { code: "INPUT_INVALID" });
  assert.throws(() => evaluate(null), { code: "INPUT_INVALID" });
  assert.throws(() => evaluate([], { fullCheck: true }), { code: "INPUT_INVALID" });
  assert.throws(() => evaluate(null, { fullCheck: true, notCollectedReason: "unknown" }), { code: "INPUT_INVALID" });
});

test("distinguishes verified empty diffs from not-collected full checks", () => {
  const empty = evaluate([]);
  assert.deepEqual(empty.changeSet, { kind: "complete", count: 0 });
  assert.equal(empty.decisions["security.secrets"].selected, true);
  const full = evaluate(null, { fullCheck: true, notCollectedReason: "manual" });
  assert.deepEqual(full.changeSet, { kind: "not-collected", reason: "manual" });
  assert.deepEqual(plainDecision(full.decisions["api.inference-mock"]), { selected: true, reason: "full-check", groups: {} });
});

test("invalidation and unmatched paths take precedence over expressions", () => {
  const invalidated = decision([{ status: "modified", path: ".github/actions/triggerd/src/main.ts" }], "foo.both-python");
  assert.equal(invalidated.reason, "global-invalidation");
  assert.equal(invalidated.selected, true);
  const unmatched = decision([{ status: "added", path: "apps/new/main.go" }], "foo.both-python");
  assert.equal(unmatched.reason, "unmatched-path");
  assert.equal(unmatched.selected, true);
});
