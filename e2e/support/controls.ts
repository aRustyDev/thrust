// The controls: ways of running a journey that MUST make it fail, so that a pass means something (AGENTS.md testing rule 8). Each is
// chosen by E2E_CONTROL, and scripts/e2e.sh (`controls`) runs it and requires the failure AND the words it fails with.
//
//   stickies-404  one route the journey calls answers 404 in the browser, as if it were not mounted. The journey must fail naming
//                 that 404: a detector that cannot see a 404 would pass every run.
//   empty-list    the browser empties the work-item list (S47) that the server answers, keeping the envelope. The journey must fail at
//                 "see it in the list": a journey that never looks at the server's list would pass with this.
//   skip          the journey skips itself, which Playwright reports as a pass (exit 0). scripts/e2e.sh's count guard must catch it.
//
// The wrong-target control is not here: it is E2E_TARGET set to the other target (support/target.ts).
//
// These are browser-side on purpose: they need no second build of the server, and they run in every cell. The real server-side
// variants (a Worker that does not mount a route, or whose S47 answers `[]`) are measured by hand and recorded in
// docs/src/testing/status.md.
import type { Page } from "@playwright/test";

export const control = process.env.E2E_CONTROL ?? "";

// the work-item list of a project (S47), with or without a query; the creation (S48) is a POST to the same path
const WORK_ITEM_LIST = /\/api\/workspaces\/[^/]+\/projects\/[0-9a-f-]+\/issues\/(\?.*)?$/;

export const STEP_LIST = "see it in the list";

export async function installControl(page: Page): Promise<{ emptied: () => number }> {
  let emptied = 0;

  if (control === "stickies-404") {
    await page.route("**/api/workspaces/*/stickies/**", (route) => route.fulfill({ status: 404, contentType: "application/json", body: "{}" }));
  }

  if (control === "empty-list") {
    await page.route(WORK_ITEM_LIST, async (route) => {
      if (route.request().method() !== "GET") return route.fallback();
      const response = await route.fetch();
      const body: unknown = await response.json().catch(() => undefined);
      // only the ungrouped page the app asks for is emptied; anything else (a refusal, a grouped answer) goes through as it came
      if (response.status() !== 200 || typeof body !== "object" || body === null || !Array.isArray((body as { results?: unknown }).results)) {
        return route.fulfill({ response });
      }
      const page_ = body as { results: unknown[] };
      if (page_.results.length > 0) emptied += 1;
      return route.fulfill({ response, json: { ...page_, results: [], count: 0, total_count: 0, total_results: 0 } });
    });
  }

  return { emptied: () => emptied };
}
