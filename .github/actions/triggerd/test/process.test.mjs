import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { createRequire } from "node:module";
import test from "node:test";

test("bounded Git process adapter exists", () => {
  assert.equal(existsSync(new URL("../.build/process.js", import.meta.url)), true);
});

const require = createRequire(import.meta.url);
const { runGit } = require("../.build/process.js");

test("Git argv is literal and successful stdout is binary-safe", async () => {
  const result = await runGit(["--version"], { timeoutMs: 1000 });
  assert.match(Buffer.from(result.stdout).toString("utf8"), /^git version /);
  await assert.rejects(() => runGit(["--version\0not-an-argument"], { timeoutMs: 1000 }), { code: "SOURCE_INVALID" });
  await assert.rejects(() => runGit(["-c", "alias.pwn=!echo pwn", "pwn"], { timeoutMs: 1000 }), { code: "SOURCE_INVALID" });
});

test("nonzero and aborted Git processes never produce a partial success", async () => {
  await assert.rejects(() => runGit(["definitely-not-a-command"], { timeoutMs: 1000 }), { code: "SOURCE_UNAVAILABLE" });
  const controller = new AbortController();
  controller.abort();
  await assert.rejects(() => runGit(["--version"], { timeoutMs: 1000, signal: controller.signal }), { code: "DEADLINE_EXCEEDED" });
});

test("nonpositive deadline is rejected before process startup", async () => {
  await assert.rejects(() => runGit(["--version"], { timeoutMs: 0 }), { code: "DEADLINE_EXCEEDED" });
});

test("timeout and output-cap terminate an injected test child", async () => {
  await assert.rejects(
    () => runGit(["-e", "setTimeout(() => {}, 1000)"], { timeoutMs: 10 }, { executable: process.execPath }),
    { code: "DEADLINE_EXCEEDED" }
  );
  await assert.rejects(
    () => runGit(["-e", "process.stdout.write('x'.repeat(64 * 1024 * 1024 + 1))"], { timeoutMs: 5000 }, { executable: process.execPath }),
    { code: "LIMIT_EXCEEDED" }
  );
});

test("Git runner passes an explicit child-only environment", async () => {
  const result = await runGit(
    ["-e", "process.stdout.write(process.env.TRIGGERD_TEST_ENV ?? '')"],
    { timeoutMs: 1000 },
    { executable: process.execPath, environment: { TRIGGERD_TEST_ENV: "child-only" } }
  );
  assert.equal(Buffer.from(result.stdout).toString("utf8"), "child-only");
});
