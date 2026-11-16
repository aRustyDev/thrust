import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { mkdtempSync, writeFileSync, unlinkSync, symlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";
import test from "node:test";

test("local Git revision and NUL diff adapter exists", () => {
  assert.equal(existsSync(new URL("../.build/git-local.js", import.meta.url)), true);
});

const require = createRequire(import.meta.url);
const { collectLocalDiff, collectPushDiff } = require("../.build/git-local.js");
const git = (repo, args) => {
  const result = spawnSync("git", ["-C", repo, ...args], { encoding: "utf8" });
  assert.equal(result.status, 0, result.stderr);
  return result.stdout.trim();
};

const repo = () => {
  const directory = mkdtempSync(join(tmpdir(), "triggerd-git-"));
  git(directory, ["init"]); git(directory, ["config", "user.email", "triggerd@example.test"]); git(directory, ["config", "user.name", "Triggerd"]); git(directory, ["config", "commit.gpgsign", "false"]);
  return directory;
};

const sourceError = async (run) => {
  await assert.rejects(run, (error) => error?.code === "SOURCE_INVALID" || error?.code === "SOURCE_UNAVAILABLE");
};

test("local adapter diffs merge-base to head", async () => {
  const directory = repo();
  writeFileSync(join(directory, "base.txt"), "base\n"); git(directory, ["add", "."]); git(directory, ["commit", "-m", "base"]);
  const base = git(directory, ["rev-parse", "HEAD"]);
  writeFileSync(join(directory, "head.txt"), "head\n"); git(directory, ["add", "."]); git(directory, ["commit", "-m", "head"]);
  const head = git(directory, ["rev-parse", "HEAD"]);
  const result = await collectLocalDiff(directory, base, head, 1000);
  assert.equal(result.mergeBase, base);
  assert.deepEqual(result.changes, [{ status: "added", path: "head.txt" }]);
  git(directory, ["rm", "head.txt"]); git(directory, ["commit", "-m", "remove"]);
  const removed = await collectLocalDiff(directory, head, git(directory, ["rev-parse", "HEAD"]), 1000);
  assert.deepEqual(removed.changes, [{ status: "removed", path: "head.txt" }]);
  writeFileSync(join(directory, "old.txt"), "rename\n"); git(directory, ["add", "."]); git(directory, ["commit", "-m", "old"]);
  const old = git(directory, ["rev-parse", "HEAD"]);
  git(directory, ["mv", "old.txt", "new.txt"]); git(directory, ["commit", "-m", "rename"]);
  const renamed = await collectLocalDiff(directory, old, git(directory, ["rev-parse", "HEAD"]), 1000);
  assert.deepEqual(renamed.changes, [{ status: "renamed", previousPath: "old.txt", path: "new.txt" }]);
  writeFileSync(join(directory, "kind.txt"), "regular\n"); git(directory, ["add", "."]); git(directory, ["commit", "-m", "regular"]);
  const regular = git(directory, ["rev-parse", "HEAD"]);
  unlinkSync(join(directory, "kind.txt")); symlinkSync("base.txt", join(directory, "kind.txt")); git(directory, ["add", "."]); git(directory, ["commit", "-m", "symlink"]);
  const typed = await collectLocalDiff(directory, regular, git(directory, ["rev-parse", "HEAD"]), 1000);
  assert.deepEqual(typed.changes, [{ status: "type-changed", path: "kind.txt" }]);
});

test("local adapter uses merge-base to head rather than a two-tree diff", async () => {
  const directory = repo();
  writeFileSync(join(directory, "shared.txt"), "root\n"); git(directory, ["add", "."]); git(directory, ["commit", "-m", "root"]);
  const root = git(directory, ["rev-parse", "HEAD"]);
  const defaultBranch = git(directory, ["symbolic-ref", "--short", "HEAD"]);
  git(directory, ["checkout", "-b", "feature"]);
  writeFileSync(join(directory, "feature.txt"), "feature\n"); git(directory, ["add", "."]); git(directory, ["commit", "-m", "feature"]);
  const feature = git(directory, ["rev-parse", "HEAD"]);
  git(directory, ["checkout", defaultBranch]);
  writeFileSync(join(directory, "shared.txt"), "advanced base\n"); git(directory, ["add", "."]); git(directory, ["commit", "-m", "advance base"]);
  const advancedBase = git(directory, ["rev-parse", "HEAD"]);
  const result = await collectLocalDiff(directory, advancedBase, feature, 1000);
  assert.equal(result.mergeBase, root);
  assert.deepEqual(result.changes, [{ status: "added", path: "feature.txt" }]);
});

test("push adapter uses the exact before-to-after range rather than merge-base", async () => {
  const directory = repo();
  writeFileSync(join(directory, "shared.txt"), "root\n"); git(directory, ["add", "."]); git(directory, ["commit", "-m", "root"]);
  const root = git(directory, ["rev-parse", "HEAD"]);
  const defaultBranch = git(directory, ["symbolic-ref", "--short", "HEAD"]);
  git(directory, ["checkout", "-b", "feature"]); writeFileSync(join(directory, "feature.txt"), "feature\n"); git(directory, ["add", "."]); git(directory, ["commit", "-m", "feature"]);
  const feature = git(directory, ["rev-parse", "HEAD"]);
  git(directory, ["checkout", defaultBranch]); writeFileSync(join(directory, "shared.txt"), "advanced\n"); git(directory, ["add", "."]); git(directory, ["commit", "-m", "advanced"]);
  const advanced = git(directory, ["rev-parse", "HEAD"]);
  assert.deepEqual((await collectPushDiff(directory, advanced, feature, 1000)).changes, [
    { status: "added", path: "feature.txt" }, { status: "modified", path: "shared.txt" }
  ]);
  assert.equal((await collectPushDiff(directory, root, root, 1000)).changes.length, 0);
});

test("local adapter preserves literal unusual paths and net reverts", async () => {
  const directory = repo();
  writeFileSync(join(directory, "base.txt"), "base\n"); git(directory, ["add", "."]); git(directory, ["commit", "-m", "base"]);
  const base = git(directory, ["rev-parse", "HEAD"]);
  const unusual = "literal\tname\n[brackets].txt";
  writeFileSync(join(directory, unusual), "unusual\n"); git(directory, ["add", "."]); git(directory, ["commit", "-m", "unusual"]);
  const unusualHead = git(directory, ["rev-parse", "HEAD"]);
  assert.deepEqual((await collectLocalDiff(directory, base, unusualHead, 1000)).changes, [{ status: "added", path: unusual }]);
  git(directory, ["rm", unusual]); git(directory, ["commit", "-m", "revert unusual"]);
  assert.deepEqual((await collectLocalDiff(directory, base, git(directory, ["rev-parse", "HEAD"]), 1000)).changes, []);
});

test("local adapter rejects no and multiple merge bases, missing objects, and invalid UTF-8 paths", async () => {
  const noBase = repo();
  writeFileSync(join(noBase, "root.txt"), "root\n"); git(noBase, ["add", "."]); git(noBase, ["commit", "-m", "root"]);
  const root = git(noBase, ["rev-parse", "HEAD"]);
  git(noBase, ["checkout", "--orphan", "other"]); git(noBase, ["rm", "-r", "--cached", "."]); unlinkSync(join(noBase, "root.txt")); writeFileSync(join(noBase, "other.txt"), "other\n"); git(noBase, ["add", "."]); git(noBase, ["commit", "-m", "other"]);
  await sourceError(() => collectLocalDiff(noBase, root, git(noBase, ["rev-parse", "HEAD"]), 1000));

  const multiple = repo();
  writeFileSync(join(multiple, "root.txt"), "root\n"); git(multiple, ["add", "."]); git(multiple, ["commit", "-m", "root"]);
  const defaultBranch = git(multiple, ["symbolic-ref", "--short", "HEAD"]);
  git(multiple, ["checkout", "-b", "left"]); writeFileSync(join(multiple, "left.txt"), "left\n"); git(multiple, ["add", "."]); git(multiple, ["commit", "-m", "left"]); const left = git(multiple, ["rev-parse", "HEAD"]);
  git(multiple, ["checkout", "-b", "right", defaultBranch]); writeFileSync(join(multiple, "right.txt"), "right\n"); git(multiple, ["add", "."]); git(multiple, ["commit", "-m", "right"]); const right = git(multiple, ["rev-parse", "HEAD"]);
  git(multiple, ["checkout", "left"]); git(multiple, ["merge", "--no-ff", right, "-m", "left merge"]); const leftMerge = git(multiple, ["rev-parse", "HEAD"]);
  git(multiple, ["checkout", "right"]); git(multiple, ["merge", "--no-ff", left, "-m", "right merge"]); const rightMerge = git(multiple, ["rev-parse", "HEAD"]);
  await sourceError(() => collectLocalDiff(multiple, leftMerge, rightMerge, 1000));

  const invalid = repo();
  writeFileSync(join(invalid, "base.txt"), "base\n"); git(invalid, ["add", "."]); git(invalid, ["commit", "-m", "base"]); const invalidBase = git(invalid, ["rev-parse", "HEAD"]);
  const blob = spawnSync("git", ["-C", invalid, "hash-object", "-w", "--stdin"], { input: "invalid\n", encoding: "utf8" }); assert.equal(blob.status, 0, blob.stderr);
  const indexed = spawnSync("git", ["-C", invalid, "update-index", "-z", "--add", "--index-info"], { input: Buffer.concat([Buffer.from(`100644 ${blob.stdout.trim()}\t`), Buffer.from([0xff, 0])]) }); assert.equal(indexed.status, 0, indexed.stderr?.toString());
  git(invalid, ["commit", "-m", "invalid path"]);
  await sourceError(() => collectLocalDiff(invalid, invalidBase, git(invalid, ["rev-parse", "HEAD"]), 1000));

  const shallowSource = repo();
  writeFileSync(join(shallowSource, "base.txt"), "base\n"); git(shallowSource, ["add", "."]); git(shallowSource, ["commit", "-m", "base"]); const shallowBase = git(shallowSource, ["rev-parse", "HEAD"]);
  writeFileSync(join(shallowSource, "head.txt"), "head\n"); git(shallowSource, ["add", "."]); git(shallowSource, ["commit", "-m", "head"]); const shallowHead = git(shallowSource, ["rev-parse", "HEAD"]);
  const shallow = mkdtempSync(join(tmpdir(), "triggerd-shallow-"));
  const cloned = spawnSync("git", ["clone", "--depth=1", `file://${shallowSource}`, shallow], { encoding: "utf8" }); assert.equal(cloned.status, 0, cloned.stderr);
  await sourceError(() => collectLocalDiff(shallow, shallowBase, shallowHead, 1000));
});
