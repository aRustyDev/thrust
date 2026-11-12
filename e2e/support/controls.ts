// The controls: ways of running a journey that MUST make it fail, so that a pass means something (AGENTS.md testing rule 8). Each is
// chosen by E2E_CONTROL, and scripts/e2e.sh (`controls`) runs it and requires the failure AND the words it fails with.
//
//   stickies-404  one route the journey calls answers 404 in the browser, as if it were not mounted. The journey must fail naming
//                 that 404: a detector that cannot see a 404 would pass every run.
//   empty-list    the browser empties the work-item list (S47) that the server answers, keeping the envelope. The journey must fail at
//                 "see it in the list": a journey that never looks at the server's list would pass with this.
//   skip          the journey skips itself, which Playwright reports as a pass (exit 0). scripts/e2e.sh's count guard must catch it.
//   slow-page     the odd one out: this run MUST PASS. The browser is slowed to a CI runner's pace, and the profile fetch that the
//                 workspace's home page makes as it mounts is slow as well. G-31 is the failure it stands for: the app marks the welcome
//                 tour as done in its store at the click, and keeps what a profile fetch returns after that, so a profile answer made
//                 before the click and landing after it puts the tour back for good, although the PATCH was answered 200. On a fast
//                 machine the page does not make that fetch at all, which is why the slowness is part of the control. A journey that
//                 answers the tour the moment it appears fails this run; one that lets the page settle first does not.
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

// how many times slower than this machine the browser runs under `slow-page` (E2E_CPU_RATE overrides it)
const CPU_RATE = Number(process.env.E2E_CPU_RATE ?? "6");
// the user's profile (GET), and how long at most the answer to the fetch from inside the workspace is held: more than a journey takes
// to click what is on screen, little enough to be cheap
const PROFILE = /\/api\/users\/me\/profile\/$/;
const LATE_MS = 2_000;

export async function installControl(
  page: Page,
  where: { workspaceSlug: string },
): Promise<{ emptied: () => number; delayed: () => number }> {
  let emptied = 0;
  let delayed = 0;
  let taken = false;

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

  if (control === "slow-page") {
    // Chromium's own throttle of the page's main thread (the DevTools "CPU: 6x slowdown"); it lasts as long as the page's session does
    const session = await page.context().newCDPSession(page);
    await session.send("Emulation.setCPUThrottlingRate", { rate: CPU_RATE });
    // and the first profile fetch from inside the workspace (the sign-in's and the onboarding's are made on other URLs) is held until the
    // tour is answered, and then answered as it was before: so it lands in the moment between that click and the next. A journey that
    // answers the tour only after the page has settled holds the answer past the click's moment, which is what LATE_MS is for
    const answered = new Promise<void>((resolve) => {
      page.on("request", (r) => {
        if (r.method() === "PATCH" && /\/api\/users\/me\/tour-completed\/$/.test(new URL(r.url()).pathname)) resolve();
      });
    });
    await page.route(PROFILE, async (route) => {
      const inWorkspace = new URL(route.request().frame().url()).pathname.startsWith(`/${where.workspaceSlug}/`);
      if (route.request().method() !== "GET" || taken || !inWorkspace) return route.fallback();
      taken = true;
      const response = await route.fetch();
      const body: unknown = await response.json().catch(() => undefined);
      await Promise.race([answered, new Promise((resolve) => setTimeout(resolve, LATE_MS))]);
      if (response.status() !== 200 || typeof body !== "object" || body === null || !("is_tour_completed" in body)) {
        return route.fulfill({ response });
      }
      delayed += 1;
      return route.fulfill({ response, json: { ...body, is_tour_completed: false } });
    });
  }

  return { emptied: () => emptied, delayed: () => delayed };
}
