import assert from "node:assert/strict";
import { cpSync, mkdtempSync, mkdirSync, rmSync, symlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createRequire } from "node:module";
import test from "node:test";

const require = createRequire(import.meta.url);
const { executeAction } = require("../.build/action.js");
const sha = (digit) => digit.repeat(40);
const policy = new URL("./fixtures/triggerd.yml", import.meta.url);

function fixture(options = {}) {
  const cwd = mkdtempSync(join(tmpdir(), "triggerd-action-"));
  mkdirSync(join(cwd, ".github"));
  cpSync(policy, join(cwd, ".github", "triggerd.yml"));
  const calls = [];
  const outputs = [];
  const warnings = [];
  const inputs = { "rules-file": ".github/triggerd.yml", scope: "", "github-token": "token-value", "full-check": "false", ...options.inputs };
  const core = {
    getInput: (name) => inputs[name] ?? "",
    setSecret: (value) => calls.push(["secret", value]),
    setOutput: (name, value) => { calls.push(["output", name]); outputs.push([name, value]); if (options.failOutput === name) throw new Error("sink failed"); },
    warning: (value) => warnings.push(value),
    summary: { addRaw: () => ({ write: async () => { if (options.failSummary) throw new Error("summary failed"); } }) }
  };
  const context = options.context ?? { eventName: "workflow_dispatch", sha: sha("3"), payload: { repository: { full_name: "aRustyDev/thrust" } } };
  const deps = {
    core,
    context,
    cwd,
    getOctokit: () => ({ rest: { pulls: {
      get: async () => ({ data: { base: { sha: sha("1") }, head: { sha: sha("2") }, changed_files: 0 } }),
      listFiles: async () => ({ data: [] })
    } } })
  };
  return { cwd, calls, outputs, warnings, deps };
}

function cleanup(item) { rmSync(item.cwd, { recursive: true, force: true }); }

test("masks the token before effects and writes complete action outputs", async () => {
  const item = fixture();
  try {
    await executeAction(item.deps);
    assert.equal(item.calls[0][0], "secret");
    assert.deepEqual(item.outputs.map(([name]) => name), ["rules", "details"]);
    assert.match(item.outputs[0][1], /^\{/);
    assert.match(item.outputs[1][1], /"mode":"full"/);
  } finally { cleanup(item); }
});

test("rejects strict inputs and escaped rules without publishing outputs", async () => {
  const invalid = fixture({ inputs: { "full-check": " true " } });
  try {
    await assert.rejects(executeAction(invalid.deps), { code: "INPUT_INVALID" });
    assert.deepEqual(invalid.outputs, []);
  } finally { cleanup(invalid); }
  const escaped = fixture();
  try {
    symlinkSync("/etc/hosts", join(escaped.cwd, ".github", "rules-link.yml"));
    escaped.deps.core.getInput = (name) => name === "rules-file" ? ".github/rules-link.yml" : name === "github-token" ? "token-value" : "";
    await assert.rejects(executeAction(escaped.deps), { code: "SOURCE_INVALID" });
    assert.deepEqual(escaped.outputs, []);
  } finally { cleanup(escaped); }
  const absolute = fixture();
  try {
    absolute.deps.core.getInput = (name) => name === "rules-file" ? join(absolute.cwd, ".github", "triggerd.yml") : name === "github-token" ? "token-value" : "";
    await assert.rejects(executeAction(absolute.deps), { code: "INPUT_INVALID" });
    assert.deepEqual(absolute.outputs, []);
  } finally { cleanup(absolute); }
});

test("fails a partial machine sink but warns for a summary-only failure", async () => {
  const partial = fixture({ failOutput: "details" });
  try {
    await assert.rejects(executeAction(partial.deps));
    assert.deepEqual(partial.outputs.map(([name]) => name), ["rules", "details"]);
  } finally { cleanup(partial); }
  const summary = fixture({ failSummary: true });
  try {
    await executeAction(summary.deps);
    assert.deepEqual(summary.outputs.map(([name]) => name), ["rules", "details"]);
    assert.deepEqual(summary.warnings, ["triggerd: summary unavailable"]);
  } finally { cleanup(summary); }
});

test("collects PR data against the event identity rather than checkout revision", async () => {
  const item = fixture({ context: {
    eventName: "pull_request", sha: sha("3"),
    payload: { action: "synchronize", number: 7, repository: { full_name: "aRustyDev/thrust" }, pull_request: { number: 7, changed_files: 0, base: { sha: sha("1") }, head: { sha: sha("2") } } }
  } });
  try {
    await executeAction(item.deps);
    const details = JSON.parse(item.outputs.find(([name]) => name === "details")[1]);
    assert.equal(details.source.head, sha("2"));
    assert.equal(details.source.testRevision, sha("3"));
    assert.equal(details.source.adapter, "pr-api");
  } finally { cleanup(item); }
});

test("uses Git history only for a complete PR API cap and brackets it with fresh metadata", async () => {
  const item = fixture({ context: {
    eventName: "pull_request", sha: sha("3"),
    payload: { action: "synchronize", number: 7, repository: { full_name: "aRustyDev/thrust" }, pull_request: { number: 7, changed_files: 3001, base: { sha: sha("1") }, head: { sha: sha("2") } } }
  } });
  let historyCalls = 0;
  item.deps.fetchHistory = async (request, use) => {
    historyCalls += 1;
    assert.equal(request.base, sha("1")); assert.equal(request.head, sha("2"));
    return use("/run-owned-bare");
  };
  item.deps.collectLocalDiff = async (bare, base, head) => {
    assert.equal(bare, "/run-owned-bare"); assert.equal(base, sha("1")); assert.equal(head, sha("2"));
    return { mergeBase: sha("1"), changes: [] };
  };
  item.deps.getOctokit = () => ({ rest: { pulls: {
    get: async () => ({ data: { base: { sha: sha("1") }, head: { sha: sha("2") }, changed_files: 3001 } }),
    listFiles: async () => { throw new Error("API file list must not run beyond the cap"); }
  } } });
  try {
    await executeAction(item.deps);
    assert.equal(historyCalls, 1);
    const details = JSON.parse(item.outputs.find(([name]) => name === "details")[1]);
    assert.equal(details.source.adapter, "pr-git");
    assert.equal(details.source.mergeBase, sha("1"));
  } finally { cleanup(item); }
});

test("never invokes PR Git fallback for stale or denied API inputs", async () => {
  for (const get of [
    async () => ({ data: { base: { sha: sha("4") }, head: { sha: sha("2") }, changed_files: 0 } }),
    async () => { const error = new Error("denied"); error.status = 403; throw error; }
  ]) {
    const item = fixture({ context: {
      eventName: "pull_request", sha: sha("3"),
      payload: { action: "synchronize", number: 7, repository: { full_name: "aRustyDev/thrust" }, pull_request: { number: 7, changed_files: 0, base: { sha: sha("1") }, head: { sha: sha("2") } } }
    } });
    let historyCalls = 0;
    item.deps.fetchHistory = async () => { historyCalls += 1; throw new Error("must not fetch"); };
    item.deps.getOctokit = () => ({ rest: { pulls: { get, listFiles: async () => ({ data: [] }) } } });
    try {
      await assert.rejects(() => executeAction(item.deps), (error) => error?.code === "SOURCE_STALE" || error?.code === "AUTH_DENIED");
      assert.equal(historyCalls, 0);
      assert.deepEqual(item.outputs, []);
    } finally { cleanup(item); }
  }
});

test("rejects a PR that changes while Git fallback runs and never publishes a failed fallback", async () => {
  const context = {
    eventName: "pull_request", sha: sha("3"),
    payload: { action: "synchronize", number: 7, repository: { full_name: "aRustyDev/thrust" }, pull_request: { number: 7, changed_files: 3001, base: { sha: sha("1") }, head: { sha: sha("2") } } }
  };
  const changed = fixture({ context });
  let reads = 0;
  changed.deps.getOctokit = () => ({ rest: { pulls: {
    get: async () => ({ data: { base: { sha: reads++ >= 2 ? sha("4") : sha("1") }, head: { sha: sha("2") }, changed_files: 3001 } }),
    listFiles: async () => { throw new Error("must not list past cap"); }
  } } });
  changed.deps.fetchHistory = async (_request, use) => use("/run-owned-bare");
  changed.deps.collectLocalDiff = async () => ({ mergeBase: sha("1"), changes: [] });
  try {
    await assert.rejects(() => executeAction(changed.deps), { code: "SOURCE_STALE" });
    assert.deepEqual(changed.outputs, []);
  } finally { cleanup(changed); }

  const failed = fixture({ context });
  failed.deps.getOctokit = () => ({ rest: { pulls: {
    get: async () => ({ data: { base: { sha: sha("1") }, head: { sha: sha("2") }, changed_files: 3001 } }),
    listFiles: async () => { throw new Error("must not list past cap"); }
  } } });
  failed.deps.fetchHistory = async () => { const error = new Error("history unavailable"); error.code = "SOURCE_UNAVAILABLE"; throw error; };
  try {
    await assert.rejects(() => executeAction(failed.deps), { code: "SOURCE_UNAVAILABLE" });
    assert.deepEqual(failed.outputs, []);
  } finally { cleanup(failed); }
});

test("collects a non-deleted push from its exact before-to-after history", async () => {
  const item = fixture({ context: {
    eventName: "push", sha: sha("2"),
    payload: { ref: "refs/heads/main", before: sha("1"), after: sha("2"), deleted: false, repository: { full_name: "aRustyDev/thrust" } }
  } });
  item.deps.fetchHistory = async (request, use) => {
    assert.equal(request.base, sha("1")); assert.equal(request.head, sha("2"));
    return use("/run-owned-bare");
  };
  item.deps.collectPushDiff = async (bare, before, head) => {
    assert.equal(bare, "/run-owned-bare"); assert.equal(before, sha("1")); assert.equal(head, sha("2"));
    return { changes: [] };
  };
  try {
    await executeAction(item.deps);
    const details = JSON.parse(item.outputs.find(([name]) => name === "details")[1]);
    assert.equal(details.source.adapter, "git");
    assert.equal(details.changeSet.kind, "complete");
  } finally { cleanup(item); }
});
