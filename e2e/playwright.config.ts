// The journeys run against a target that scripts/e2e.sh has up and that serves the built app (a Worker started by scripts/contract.sh's
// `e2e` pass, or the pinned Plane); the address comes from THRUST_BASE_URL and E2E_TARGET says which target it is. One spec per
// journey, in journeys/; support/ holds what they share and is not matched as a spec. One worker, no retries: a journey that passes on
// its second try is a journey that fails sometimes (AGENTS.md testing rule 2 says the same of the Rust suites), and one browser on one
// database is not a thing to parallelise.
import { defineConfig, type ReporterDescription } from "@playwright/test";

const baseURL = process.env.THRUST_BASE_URL;
if (!baseURL) {
  throw new Error("THRUST_BASE_URL is not set: start a target with scripts/e2e.sh <target>, which sets it");
}

// scripts/e2e.sh asks for the run's own report (E2E_JSON) and reads its counts: a journey that was skipped exits 0 here
const reporter: ReporterDescription[] = [["list"]];
if (process.env.E2E_JSON) reporter.push(["json", { outputFile: process.env.E2E_JSON }]);

export default defineConfig({
  testDir: "journeys",
  testMatch: "*.spec.ts",
  fullyParallel: false,
  workers: 1,
  retries: 0,
  forbidOnly: true,
  // on the pinned Plane a throttled sign-in call can be held up to a minute (support/throttle.ts), and the page waits for its answer
  timeout: process.env.E2E_TARGET === "plane" ? 240_000 : 120_000,
  expect: { timeout: process.env.E2E_TARGET === "plane" ? 90_000 : 30_000 },
  reporter,
  outputDir: process.env.E2E_OUT ? `${process.env.E2E_OUT}/test-results` : "test-results",
  use: {
    baseURL,
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },
  projects: [{ name: "chromium", use: { browserName: "chromium" } }],
});
