// The first journey runs against a target that scripts/contract.sh (its `e2e` pass) has started and that serves the built app; the
// address comes from THRUST_BASE_URL. One worker, no retries: a journey that passes on its second try is a journey that fails
// sometimes (AGENTS.md testing rule 2 says the same of the Rust suites), and one browser on one database is not a thing to
// parallelise.
import { defineConfig } from "@playwright/test";

const baseURL = process.env.THRUST_BASE_URL;
if (!baseURL) {
  throw new Error("THRUST_BASE_URL is not set: start a target with scripts/e2e.sh <target>, which sets it");
}

export default defineConfig({
  testDir: ".",
  testMatch: "*.spec.ts",
  fullyParallel: false,
  workers: 1,
  retries: 0,
  forbidOnly: true,
  timeout: 120_000,
  expect: { timeout: 30_000 },
  reporter: [["list"]],
  outputDir: process.env.E2E_OUT ? `${process.env.E2E_OUT}/test-results` : "test-results",
  use: {
    baseURL,
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },
  projects: [{ name: "chromium", use: { browserName: "chromium" } }],
});
