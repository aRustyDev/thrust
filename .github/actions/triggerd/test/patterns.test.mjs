import assert from "node:assert/strict";
import { createRequire } from "node:module";
import test from "node:test";

const require = createRequire(import.meta.url);
const { compilePattern } = require("../.build/patterns.js");
const { LIMITS } = require("../.build/limits.js");

test("matches direct, nested, dotfile and Unicode paths with fixed POSIX semantics", () => {
  assert.equal(compilePattern("src/foo/*.py")("src/foo/a.py"), true);
  assert.equal(compilePattern("src/foo/*.py")("src/foo/nested/a.py"), false);
  assert.equal(compilePattern("src/foo/**/*.py")("src/foo/nested/a.py"), true);
  assert.equal(compilePattern("**/.env")("x/.env"), true);
  assert.equal(compilePattern("café/*.py")("café/a.py"), true);
  assert.equal(compilePattern("docs/*.md")("docs/guide with spaces.md"), true);
  assert.equal(compilePattern("docs/**")("docs/line\nbreak.md"), true);
  assert.equal(compilePattern("src/*.py")("SRC/a.py"), false);
});

for (const pattern of ["", "../a", "/a", "./a", "a/./b", "a/../b", "a\\b", "a/**x", "a/***", "a/{b,c}", "a/[bc]", "a/@(b)", "a/+(b)", "a/?(b)", "a/!(b)", "!a", "a//b", "a/\0b"]) test(`rejects unsupported ${JSON.stringify(pattern)}`, () => assert.throws(() => compilePattern(pattern), { code: "CONFIG_INVALID" }));

test("enforces pattern byte, segment and globstar ceilings", () => {
  assert.doesNotThrow(() => compilePattern("a".repeat(LIMITS.patternBytes)));
  assert.throws(() => compilePattern("a".repeat(LIMITS.patternBytes + 1)), { code: "CONFIG_INVALID" });
  assert.doesNotThrow(() => compilePattern(Array.from({ length: LIMITS.patternSegments }, () => "a").join("/")));
  assert.throws(() => compilePattern(Array.from({ length: LIMITS.patternSegments + 1 }, () => "a").join("/")), { code: "CONFIG_INVALID" });
  assert.doesNotThrow(() => compilePattern(Array.from({ length: LIMITS.globstars }, () => "**").join("/")));
  assert.throws(() => compilePattern(Array.from({ length: LIMITS.globstars + 1 }, () => "**").join("/")), { code: "CONFIG_INVALID" });
});

test("does not silently cap matching below validated path bounds", () => {
  assert.equal(compilePattern("**")("a".repeat(LIMITS.pathBytes)), true);
});
