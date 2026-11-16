import { runGit } from "./process.js";
import { TriggerdError } from "./errors.js";
import { normalizeChanges, type ChangeRecord } from "./changes.js";

const sha = /^[0-9a-f]{40}$/;
const fail = (): never => { throw new TriggerdError("SOURCE_INVALID", "Local Git source invalid"); };
const decoder = new TextDecoder("utf-8", { fatal: true });

function decode(bytes: Uint8Array): string {
  try { return decoder.decode(bytes); } catch { return fail(); }
}

function parseMergeBase(bytes: Uint8Array): string {
  const match = /^([0-9a-f]{40})\n$/.exec(decode(bytes));
  return match ? match[1] : fail();
}

function parseNameStatus(bytes: Uint8Array): readonly ChangeRecord[] {
  if (bytes.length === 0) return Object.freeze([]);
  const output = decode(bytes);
  if (!output.endsWith("\0")) fail();
  const fields = output.slice(0, -1).split("\0");
  const changes: ChangeRecord[] = [];
  for (let index = 0; index < fields.length;) {
    const status = fields[index]; const path = fields[index + 1];
    if (path === undefined) fail();
    if (/^[RC]\d+$/.test(status)) {
      const destination = fields[index + 2];
      if (destination === undefined) fail();
      changes.push({ status: status.startsWith("R") ? "renamed" : "copied", path: destination, previousPath: path }); index += 3; continue;
    }
    if (!/^[AMDT]$/.test(status)) fail();
    changes.push({ status: status === "A" ? "added" : status === "D" ? "removed" : status === "T" ? "type-changed" : "modified", path }); index += 2;
  }
  try { return normalizeChanges(changes); } catch { return fail(); }
}

export async function collectLocalDiff(repo: string, base: string, head: string, timeoutMs: number): Promise<Readonly<{ mergeBase: string; changes: readonly ChangeRecord[] }>> {
  if (!repo || !sha.test(base) || !sha.test(head)) fail();
  const args = ["-C", repo];
  const mergeBase = parseMergeBase((await runGit([...args, "merge-base", "--all", base, head], { timeoutMs })).stdout);
  const changes = parseNameStatus((await runGit([...args, "diff", "--no-ext-diff", "--no-textconv", "--ignore-submodules=all", "--find-renames", "--name-status", "-z", mergeBase, head], { timeoutMs })).stdout);
  return Object.freeze({ mergeBase, changes });
}

export async function collectPushDiff(repo: string, before: string, head: string, timeoutMs: number): Promise<Readonly<{ changes: readonly ChangeRecord[] }>> {
  if (!repo || !sha.test(before) || !sha.test(head)) fail();
  const changes = parseNameStatus((await runGit(["-C", repo, "diff", "--no-ext-diff", "--no-textconv", "--ignore-submodules=all", "--find-renames", "--name-status", "-z", before, head], { timeoutMs })).stdout);
  return Object.freeze({ changes });
}
