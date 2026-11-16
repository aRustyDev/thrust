import assert from "node:assert/strict";
import { existsSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createRequire } from "node:module";
import test from "node:test";

const require = createRequire(import.meta.url);
const { withFetchedHistory } = require("../.build/git-fetch.js");
const sha = (character) => character.repeat(40);
const request = Object.freeze({ owner: "aRustyDev", repository: "thrust", base: sha("a"), head: sha("b"), token: "fake-token-must-not-leak", timeoutMs: 10_000 });

function fixture(options = {}) {
  const root = mkdtempSync(join(tmpdir(), "triggerd-fetch-"));
  const calls = [];
  const removed = [];
  const runGit = async (argv, processOptions, dependencies) => {
    calls.push({ argv, timeoutMs: processOptions.timeoutMs, environment: dependencies.environment });
    if (options.failFetch && argv.includes("fetch")) throw new Error(request.token);
    if (options.missingObject && argv.includes("cat-file")) throw new Error("missing object");
    return Object.freeze({ stdout: new Uint8Array() });
  };
  return {
    calls,
    removed,
    root,
    dependencies: {
      runGit,
      makeTemporary: () => mkdtempSync(join(root, "bare-")),
      remove: (path) => { removed.push(path); rmSync(path, { recursive: true, force: true }); },
      environment: { PATH: "/verified/bin", GIT_CONFIG_GLOBAL: "/hostile", GIT_CONFIG_PARAMETERS: "credential.helper=!pwn", GIT_ALLOW_PROTOCOL: "ext", GIT_SSH_COMMAND: "pwn", GIT_ASKPASS: "pwn", GIT_SSL_NO_VERIFY: "true", SSL_CERT_FILE: "/certs/ca.pem" },
      now: () => 1_000
    }
  };
}

test("fetch adapter constructs only a GitHub HTTPS origin and a sanitized child environment", async () => {
  const local = fixture();
  try {
    const result = await withFetchedHistory(request, async (bare) => ({ bare, value: "ok" }), local.dependencies);
    assert.equal(result.value, "ok");
    assert.equal(local.calls.length, 4);
    const fetch = local.calls.find((call) => call.argv.includes("fetch"));
    assert.deepEqual(fetch.argv, ["-C", result.bare, "fetch", "--no-tags", "--depth=64", "https://github.com/aRustyDev/thrust.git", `+${request.base}:refs/triggerd/base`, `+${request.head}:refs/triggerd/head`]);
    for (const call of local.calls) {
      assert.equal(call.timeoutMs, request.timeoutMs);
      assert.equal(call.environment.PATH, "/verified/bin");
      assert.equal(call.environment.SSL_CERT_FILE, "/certs/ca.pem");
      assert.equal(call.environment.GIT_CONFIG_GLOBAL, "/dev/null");
      assert.equal(call.environment.GIT_CONFIG_NOSYSTEM, "1");
      assert.equal(call.environment.GIT_ALLOW_PROTOCOL, "https");
      assert.equal(call.environment.GIT_TERMINAL_PROMPT, "0");
      assert.equal(call.environment.GIT_CONFIG_KEY_0, "http.https://github.com/.extraheader");
      assert.equal(call.environment.GIT_CONFIG_VALUE_0, `AUTHORIZATION: basic ${Buffer.from(`x-access-token:${request.token}`).toString("base64")}`);
      for (const name of ["GIT_CONFIG_PARAMETERS", "GIT_SSH_COMMAND", "GIT_ASKPASS", "GIT_SSL_NO_VERIFY"]) assert.equal(Object.hasOwn(call.environment, name), false);
    }
    assert.equal(JSON.stringify(local.calls).includes(request.token), false);
    assert.equal(local.removed.length, 1);
    assert.equal(existsSync(result.bare), false);
  } finally { rmSync(local.root, { recursive: true, force: true }); }
});

test("fetch adapter rejects spoofed origin input and never leaves a bare repository on failure", async () => {
  const invalid = fixture();
  try {
    await assert.rejects(() => withFetchedHistory({ ...request, owner: "github.com@evil" }, async () => "never", invalid.dependencies), { code: "SOURCE_INVALID" });
    assert.equal(invalid.calls.length, 0);

    const interrupted = fixture({ failFetch: true });
    try {
      await assert.rejects(() => withFetchedHistory(request, async () => "never", interrupted.dependencies), (error) => error?.code === "SOURCE_UNAVAILABLE" && !error.message.includes(request.token));
      assert.equal(interrupted.removed.length, 1);
      assert.equal(existsSync(interrupted.removed[0]), false);
    } finally { rmSync(interrupted.root, { recursive: true, force: true }); }
  } finally { rmSync(invalid.root, { recursive: true, force: true }); }
});

test("cleanup failure cannot replace a successful fetched-history result", async () => {
  const local = fixture();
  local.dependencies.remove = (path) => { local.removed.push(path); rmSync(path, { recursive: true, force: true }); throw new Error("cleanup unavailable"); };
  try {
    assert.equal(await withFetchedHistory(request, async () => "completed", local.dependencies), "completed");
    assert.equal(local.removed.length, 1);
  } finally { rmSync(local.root, { recursive: true, force: true }); }
});

test("fetch adapter deepens only bounded ancestry attempts and rejects missing exact objects", async () => {
  const exhausted = fixture();
  try {
    await assert.rejects(() => withFetchedHistory(request, async () => { const error = new Error("unavailable"); error.code = "SOURCE_UNAVAILABLE"; throw error; }, exhausted.dependencies), { code: "SOURCE_UNAVAILABLE" });
    assert.deepEqual(exhausted.calls.filter((call) => call.argv.includes("fetch")).map((call) => call.argv[4]), ["--depth=64", "--depth=256", "--depth=1024"]);
    assert.equal(exhausted.removed.length, 1);

    const missing = fixture({ missingObject: true });
    try {
      await assert.rejects(() => withFetchedHistory(request, async () => "never", missing.dependencies), { code: "SOURCE_UNAVAILABLE" });
      assert.equal(missing.calls.filter((call) => call.argv.includes("fetch")).length, 1);
      assert.equal(missing.removed.length, 1);
    } finally { rmSync(missing.root, { recursive: true, force: true }); }
  } finally { rmSync(exhausted.root, { recursive: true, force: true }); }
});

test("fetch adapter gives every child the remaining shared deadline and cleans up on expiry", async () => {
  const local = fixture();
  let now = 0;
  local.dependencies.now = () => now;
  local.dependencies.runGit = async (argv, options, dependencies) => {
    local.calls.push({ argv, timeoutMs: options.timeoutMs, environment: dependencies.environment });
    now += 4;
    return Object.freeze({ stdout: new Uint8Array() });
  };
  try {
    await assert.rejects(() => withFetchedHistory({ ...request, timeoutMs: 5 }, async () => "never", local.dependencies), { code: "DEADLINE_EXCEEDED" });
    assert.deepEqual(local.calls.map((call) => call.timeoutMs), [5, 1]);
    assert.equal(local.removed.length, 1);
  } finally { rmSync(local.root, { recursive: true, force: true }); }
});
