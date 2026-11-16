import { mkdtempSync, readdirSync, readFileSync, rmSync, statSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { spawnSync } from "node:child_process";

const root = resolve(import.meta.dirname, "..");
const index = process.argv.indexOf("--dist");
const dist = index === -1 ? join(root, "dist") : resolve(root, process.argv[index + 1] ?? "");
if ((index !== -1 && index !== process.argv.length - 2) || !dist.startsWith(`${root}/`)) throw new Error("invalid dist directory");
const temporary = mkdtempSync(join(root, ".check-dist-"));
try {
  const result = spawnSync(process.execPath, [join(root, "scripts", "build.mjs"), "--out", relative(root, temporary)], { cwd: root, encoding: "utf8" });
  if (result.status !== 0) throw new Error("distribution rebuild failed");
  const files = (directory) => readdirSync(directory, { recursive: true }).filter((name) => statSync(join(directory, name)).isFile()).sort();
  const expected = files(temporary);
  const actual = files(dist);
  if (JSON.stringify(actual) !== JSON.stringify(expected)) throw new Error("distribution file set differs");
  for (const file of expected) if (!readFileSync(join(temporary, file)).equals(readFileSync(join(dist, file)))) throw new Error(`distribution differs: ${file}`);
} finally { rmSync(temporary, { recursive: true, force: true }); }
