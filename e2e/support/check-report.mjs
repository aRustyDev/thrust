// The count guard of scripts/e2e.sh: read Playwright's own JSON report of a run and require that every journey ran and passed.
//
//   node support/check-report.mjs <report.json> <number of journeys>
//
// Exit 0 only when `expected` (Playwright's word for a test that passed as it should) equals the number of journeys, and nothing was
// unexpected, skipped or flaky. The number of journeys comes from the files in e2e/journeys/, not from the run, so that a filter, a
// `test.skip` or a spec that registers nothing cannot select fewer and report green: Playwright exits 0 for a skipped test.
//
// It also prints one `e2e: NOTE: tour-came-back ...` line for each journey that recorded that annotation, which is not a verdict (the exit
// status does not depend on it): the journey answers the welcome tour again when the page put it back and goes on, so a pass would
// otherwise say nothing about it. The annotation is in the JSON report only, never in the console (G-31, "the race").
import { readFileSync } from "node:fs";

// The notes of every journey in a report's suites (they nest by file and by describe block), as `"<journey>": <what it recorded>`.
function tourCameBack(suite, found = []) {
  for (const inner of suite?.suites ?? []) tourCameBack(inner, found);
  for (const spec of suite?.specs ?? []) {
    for (const test of spec.tests ?? []) {
      for (const note of test.annotations ?? []) {
        if (note.type === "tour-came-back") found.push(`"${spec.title}": ${note.description ?? ""}`);
      }
    }
  }
  return found;
}

const [report, journeys] = process.argv.slice(2);
const want = Number(journeys);
if (!report || !Number.isInteger(want) || want < 1) {
  console.error("e2e: COUNT GUARD: usage: check-report.mjs <report.json> <number of journeys, at least 1>");
  process.exit(2);
}

let stats;
let notes = [];
try {
  const parsed = JSON.parse(readFileSync(report, "utf8"));
  stats = parsed.stats;
  for (const suite of parsed.suites ?? []) notes = tourCameBack(suite, notes);
} catch (error) {
  console.error(`e2e: COUNT GUARD: no readable report at ${report}: ${error.message}`);
  process.exit(1);
}

const problems = [];
if (stats?.expected !== want) problems.push(`${stats?.expected} journeys passed, and there are ${want}`);
for (const key of ["unexpected", "skipped", "flaky"]) if (stats?.[key] !== 0) problems.push(`${key} is ${stats?.[key]}, not 0`);

console.log(`e2e: report ${report}: passed ${stats?.expected} of ${want}; unexpected ${stats?.unexpected}, skipped ${stats?.skipped}, flaky ${stats?.flaky}`);
for (const note of notes) {
  console.log(`e2e: NOTE: tour-came-back in ${report}, ${note}: the tour was back and the journey answered it again, so the race fired (G-31); expected under the slow-page control, and on the main run it is the race happening there`);
}
if (problems.length > 0) {
  console.error(`e2e: COUNT GUARD: ${problems.join("; ")}`);
  process.exit(1);
}
