import assert from "node:assert/strict";
import { createRequire } from "node:module";
import test from "node:test";

const require = createRequire(import.meta.url);
const { collectPrChanges, createGithubPrClient } = require("../.build/github.js");
const sha = (digit) => digit.repeat(40);
const metadata = { base: sha("1"), head: sha("2"), changedFiles: 101 };
const file = (index) => ({ status: "modified", filename: `src/f${index}.ts` });

test("collects 101 API rows at 100/page and verifies metadata before and after", async () => {
  const calls = [];
  const client = {
    metadata: async () => metadata,
    files: async (page) => { calls.push(page); return page === 1 ? Array.from({ length: 100 }, (_, index) => file(index)) : [file(100)]; }
  };
  const changes = await collectPrChanges(client, metadata);
  assert.equal(changes.length, 101);
  assert.deepEqual(calls, [1, 2]);
});

test("maps GitHub changed rows to type-changed records", async () => {
  const stable = { ...metadata, changedFiles: 1 };
  const client = {
    metadata: async () => stable,
    files: async () => [{ status: "changed", filename: "scripts/executable" }]
  };
  assert.deepEqual(await collectPrChanges(client, stable), [{ status: "type-changed", path: "scripts/executable" }]);
});

for (const count of [0, 1, 100, 3000]) test(`collects exactly ${count} rows when metadata proves completeness`, async () => {
  const stable = { ...metadata, changedFiles: count };
  const client = {
    metadata: async () => stable,
    files: async (page) => Array.from({ length: Math.min(100, Math.max(0, count - (page - 1) * 100)) }, (_, index) => file((page - 1) * 100 + index))
  };
  assert.equal((await collectPrChanges(client, stable)).length, count);
});

test("accepts a final full page only when the exact 3000 count remains stable", async () => {
  const stable = { ...metadata, changedFiles: 3000 };
  const pages = [];
  const client = {
    metadata: async () => stable,
    files: async (page) => { pages.push(page); return Array.from({ length: 100 }, (_, index) => file((page - 1) * 100 + index)); }
  };
  assert.equal((await collectPrChanges(client, stable)).length, 3000);
  assert.deepEqual(pages, Array.from({ length: 30 }, (_, index) => index + 1));
});

for (const [name, client, code] of [
  ["cap overflow", { metadata: async () => ({ ...metadata, changedFiles: 3001 }), files: async () => [] }, "SOURCE_INCOMPLETE"],
  ["metadata movement", { metadata: (() => { let call = 0; return async () => (++call === 1 ? metadata : { ...metadata, head: sha("3") }); })(), files: async () => [file(0)] }, "SOURCE_STALE"],
  ["malformed row", { metadata: async () => ({ ...metadata, changedFiles: 1 }), files: async () => [{ status: "unknown", filename: "a" }] }, "SOURCE_INVALID"]
]) test(`rejects ${name}`, async () => await assert.rejects(collectPrChanges(client, name === "cap overflow" ? { ...metadata, changedFiles: 3001 } : name === "malformed row" ? { ...metadata, changedFiles: 1 } : metadata), { code }));

test("retries a transient API failure exactly three times and keeps auth terminal", async () => {
  let attempts = 0;
  const unavailable = { metadata: async () => { attempts += 1; throw { status: 503 }; }, files: async () => [] };
  await assert.rejects(collectPrChanges(unavailable, metadata), { code: "SOURCE_UNAVAILABLE" });
  assert.equal(attempts, 3);
  let authAttempts = 0;
  const denied = { metadata: async () => { authAttempts += 1; throw { status: 403 }; }, files: async () => [] };
  await assert.rejects(collectPrChanges(denied, metadata), { code: "AUTH_DENIED" });
  assert.equal(authAttempts, 1);
});

test("uses bounded retry timings and honors a longer Retry-After", async () => {
  let attempts = 0;
  let clock = 0;
  const waits = [];
  const options = {
    now: () => clock,
    wait: async (milliseconds) => { waits.push(milliseconds); clock += milliseconds; },
    retryAfter: (error) => error.retryAfterMs
  };
  const client = {
    metadata: async () => {
      attempts += 1;
      if (attempts === 1) throw { status: 503 };
      if (attempts === 2) throw { status: 429, retryAfterMs: 2_000 };
      return { ...metadata, changedFiles: 0 };
    },
    files: async () => []
  };
  assert.equal((await collectPrChanges(client, { ...metadata, changedFiles: 0 }, 10_000, 0, options)).length, 0);
  assert.deepEqual(waits, [250, 2_000]);
});

test("uses the GitHub Retry-After response header when it is longer", async () => {
  let attempts = 0;
  let clock = 0;
  const waits = [];
  const stable = { ...metadata, changedFiles: 0 };
  const client = {
    metadata: async () => {
      attempts += 1;
      if (attempts === 1) throw { status: 429, response: { headers: { "retry-after": "2" } } };
      return stable;
    },
    files: async () => []
  };
  await collectPrChanges(client, stable, 10_000, 0, {
    now: () => clock,
    wait: async (milliseconds) => { waits.push(milliseconds); clock += milliseconds; }
  });
  assert.deepEqual(waits, [2_000]);
});

test("does not begin a retry that cannot fit in the total deadline", async () => {
  let attempts = 0;
  let waits = 0;
  const client = {
    metadata: async () => { attempts += 1; throw { status: 503 }; },
    files: async () => []
  };
  await assert.rejects(collectPrChanges(client, metadata, 200, 0, {
    now: () => 0,
    wait: async () => { waits += 1; }
  }), { code: "DEADLINE_EXCEEDED" });
  assert.equal(attempts, 1);
  assert.equal(waits, 0);
});

test("retries transport timeouts but refuses an over-budget Retry-After", async () => {
  let attempts = 0;
  let clock = 0;
  const waits = [];
  const stable = { ...metadata, changedFiles: 0 };
  const recovering = {
    metadata: async () => {
      attempts += 1;
      if (attempts <= 2) throw { code: "ETIMEDOUT" };
      return stable;
    },
    files: async () => []
  };
  await collectPrChanges(recovering, stable, 10_000, 0, {
    now: () => clock,
    wait: async (milliseconds) => { waits.push(milliseconds); clock += milliseconds; }
  });
  assert.deepEqual(waits, [250, 1000]);
  let calls = 0;
  const retryAfter = { metadata: async () => { calls += 1; throw { status: 429, retryAfterMs: 2_000 }; }, files: async () => [] };
  await assert.rejects(collectPrChanges(retryAfter, metadata, 1_000, 0, {
    now: () => 0,
    retryAfter: (error) => error.retryAfterMs
  }), { code: "DEADLINE_EXCEEDED" });
  assert.equal(calls, 1);
});

test("rejects duplicate API rows as an invalid source", async () => {
  const stable = { ...metadata, changedFiles: 2 };
  const client = {
    metadata: async () => stable,
    files: async () => [file(1), file(1)]
  };
  await assert.rejects(collectPrChanges(client, stable), { code: "SOURCE_INVALID" });
});

test("adapts GitHub pull endpoints with fixed pagination and an abort signal", async () => {
  const calls = [];
  const octokit = {
    rest: { pulls: {
      get: async (request) => { calls.push(["get", request]); return { data: { base: { sha: sha("1") }, head: { sha: sha("2") }, changed_files: 1 } }; },
      listFiles: async (request) => { calls.push(["files", request]); return { data: [{ status: "modified", filename: "src/example.ts" }] }; }
    } }
  };
  const client = createGithubPrClient(octokit, "aRustyDev/thrust", 7);
  const controller = new AbortController();
  assert.deepEqual(await client.metadata(controller.signal), { base: sha("1"), head: sha("2"), changedFiles: 1 });
  assert.deepEqual(await client.files(2, controller.signal), [{ status: "modified", filename: "src/example.ts" }]);
  assert.deepEqual(calls.map(([operation, request]) => [operation, request.owner, request.repo, request.pull_number, request.page, request.per_page, request.request.signal]), [
    ["get", "aRustyDev", "thrust", 7, undefined, undefined, controller.signal],
    ["files", "aRustyDev", "thrust", 7, 2, 100, controller.signal]
  ]);
});

test("rejects malformed GitHub responses and keeps authorization errors safe", async () => {
  const malformed = createGithubPrClient({ rest: { pulls: { get: async () => ({ data: { changed_files: 1 } }), listFiles: async () => ({ data: [] }) } } }, "aRustyDev/thrust", 7);
  await assert.rejects(malformed.metadata(), { code: "SOURCE_INVALID" });
  const denied = { metadata: async () => { throw { status: 401, message: "token=top-secret" }; }, files: async () => [] };
  await assert.rejects(collectPrChanges(denied, metadata), (error) => error.code === "AUTH_DENIED" && !error.message.includes("top-secret"));
});

test("rejects an already exhausted total deadline before any API request", async () => {
  let calls = 0;
  const client = { metadata: async () => { calls += 1; return metadata; }, files: async () => [] };
  await assert.rejects(collectPrChanges(client, metadata, 1, 0), { code: "DEADLINE_EXCEEDED" });
  assert.equal(calls, 0);
});

test("aborts an in-flight request when the total deadline expires", async () => {
  let sawAbort = false;
  const client = {
    metadata: async (signal) => new Promise((_, reject) => signal.addEventListener("abort", () => { sawAbort = true; reject(new DOMException("aborted", "AbortError")); }, { once: true })),
    files: async () => []
  };
  await assert.rejects(collectPrChanges(client, metadata, 10, Date.now()), { code: "DEADLINE_EXCEEDED" });
  assert.equal(sawAbort, true);
});
