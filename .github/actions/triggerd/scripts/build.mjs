import { cpSync, mkdirSync, renameSync, rmSync } from "node:fs";
import { basename, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";

const root = resolve(import.meta.dirname, "..");
const outputIndex = process.argv.indexOf("--out");
const output = outputIndex === -1 ? join(root, "dist") : resolve(root, process.argv[outputIndex + 1] ?? "");
if ((outputIndex !== -1 && outputIndex !== process.argv.length - 2) || !output.startsWith(`${root}/`)) throw new Error("invalid output directory");
const temporary = join(root, `.build-dist-${basename(output)}`);
const ncc = join(root, "node_modules", ".bin", "ncc");
rmSync(output, { recursive: true, force: true });
rmSync(temporary, { recursive: true, force: true });
mkdirSync(temporary, { recursive: true });
for (const [entry, filename, license] of [["main.js", "main.js", true], ["worker-main.js", "worker-main.js", false], ["gate-main.js", "gate.js", false]]) {
  const destination = join(temporary, filename);
  const result = spawnSync(ncc, ["build", join(root, ".build", entry), "-o", destination, "--no-source-map-register", "--target", "es2022", "--minify", ...(license ? ["--license", "licenses.txt"] : [])], { cwd: root, encoding: "utf8" });
  if (result.status !== 0) throw new Error(result.stderr || "ncc build failed");
}
mkdirSync(output, { recursive: true });
cpSync(join(temporary, "main.js"), output, { recursive: true });
renameSync(join(output, "index.js"), join(output, "main.js"));
cpSync(join(temporary, "worker-main.js"), join(output, ".worker"), { recursive: true });
renameSync(join(output, ".worker", "index.js"), join(output, "worker-main.js"));
rmSync(join(output, ".worker"), { recursive: true, force: true });
cpSync(join(temporary, "gate.js"), join(output, ".gate"), { recursive: true });
renameSync(join(output, ".gate", "index.js"), join(output, "gate.js"));
rmSync(join(output, ".gate"), { recursive: true, force: true });
rmSync(temporary, { recursive: true, force: true });
