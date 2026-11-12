// The count guard of scripts/e2e.sh: read Playwright's own JSON report of a run and require that every journey ran and passed.
//
//   node support/check-report.mjs <report.json> <number of journeys>
//
// Exit 0 only when `expected` (Playwright's word for a test that passed as it should) equals the number of journeys, and nothing was
// unexpected, skipped or flaky. The number of journeys comes from the files in e2e/journeys/, not from the run, so that a filter, a
// `test.skip` or a spec that registers nothing cannot select fewer and report green: Playwright exits 0 for a skipped test.
import { readFileSync } from "node:fs";

const [report, journeys] = process.argv.slice(2);
const want = Number(journeys);
if (!report || !Number.isInteger(want) || want < 1) {
  console.error("e2e: COUNT GUARD: usage: check-report.mjs <report.json> <number of journeys, at least 1>");
  process.exit(2);
}

let stats;
try {
  stats = JSON.parse(readFileSync(report, "utf8")).stats;
} catch (error) {
  console.error(`e2e: COUNT GUARD: no readable report at ${report}: ${error.message}`);
  process.exit(1);
}

const problems = [];
if (stats?.expected !== want) problems.push(`${stats?.expected} journeys passed, and there are ${want}`);
for (const key of ["unexpected", "skipped", "flaky"]) if (stats?.[key] !== 0) problems.push(`${key} is ${stats?.[key]}, not 0`);

console.log(`e2e: report ${report}: passed ${stats?.expected} of ${want}; unexpected ${stats?.unexpected}, skipped ${stats?.skipped}, flaky ${stats?.flaky}`);
if (problems.length > 0) {
  console.error(`e2e: COUNT GUARD: ${problems.join("; ")}`);
  process.exit(1);
}
