import { mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { validateSummary } from "./test-summary.mjs";

const temp = mkdtempSync(join(tmpdir(), "triggerd-test-"));
const summaryPath = join(temp, "summary.json");
const reporterPath = join(temp, "reporter.mjs");
writeFileSync(reporterPath, `import { writeFileSync } from "node:fs"; export default async function* (source) { for await (const event of source) { if (event.type === "test:summary" && !event.data.file) { const counts = event.data.counts; writeFileSync(process.env.TRIGGERD_SUMMARY, JSON.stringify({type:event.type, success:event.data.success, tests:counts.tests, failed:counts.failed, cancelled:counts.cancelled, skipped:counts.skipped, todo:counts.todo})); } } }`);
const files = process.argv.slice(2);
if (files.length === 0) files.push(...readdirSync("test").filter((name) => name.endsWith(".test.mjs") && name !== "installed.test.mjs").map((name) => join("test", name)));
const result = spawnSync(process.execPath, ["--test", "--test-reporter", reporterPath, ...files], {
  env: { ...process.env, TRIGGERD_SUMMARY: summaryPath },
  encoding: "utf8"
});
let summary;
try { summary = JSON.parse(readFileSync(summaryPath, "utf8")); } catch { summary = undefined; }
rmSync(temp, { recursive: true, force: true });
const error = validateSummary(summary);
if (error) {
  process.stderr.write(`triggerd test runner: ${error}\n`);
  process.exitCode = 1;
} else if (result.status !== 0) {
  process.exitCode = result.status ?? 1;
}
