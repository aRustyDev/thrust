export function validateSummary(summary) {
  if (!summary || summary.type !== "test:summary") return "missing final test summary";
  if (summary.success !== true) return "test runner reported failure";
  if (!Number.isInteger(summary.tests) || summary.tests < 1) return "no real tests collected";
  for (const key of ["failed", "cancelled", "skipped", "todo"]) {
    if (!Number.isInteger(summary[key]) || summary[key] !== 0) return `nonzero ${key}`;
  }
  return undefined;
}
