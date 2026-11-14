// The browser context every journey runs in, and the one place its recording is configured.
import type { Browser, BrowserContext, Page } from "@playwright/test";
import { ACTION_TIMEOUT_MS, paceThePin } from "./throttle";

// The API's own calls: what spike 0009's replay harness reads. The app's JS, CSS and fonts are megabytes and say nothing a replay needs.
const API_CALLS = /\/(api|auth)\//;

export async function openJourney(browser: Browser, baseURL: string, name: string): Promise<{ context: BrowserContext; page: Page }> {
  // E2E_HAR_BODIES=1 (scripts/e2e.sh sets it for the pinned Plane, whose HAR is committed after tools/e2e-har/scrub.py has scrubbed it)
  // keeps the bodies of the API's calls. Otherwise the log is headers and statuses only: a run on a Worker, in CI, keeps no cookie
  // value and no password in an artifact.
  const bodies = process.env.E2E_HAR_BODIES === "1";
  const context = await browser.newContext({
    baseURL,
    // the app is a service-worker PWA; the journey is about the server's answers, so the worker stays out of it
    serviceWorkers: "block",
    viewport: { width: 1400, height: 950 },
    // the network log, kept. It is written when the context closes
    recordHar: {
      // scripts/e2e.sh gives each run its own directory (E2E_HAR_DIR), so that a control cannot overwrite the main run's log
      path: `${process.env.E2E_HAR_DIR ?? process.env.E2E_OUT ?? "."}/${name}.har`,
      ...(bodies ? { content: "embed" as const, mode: "full" as const, urlFilter: API_CALLS } : { content: "omit" as const }),
    },
  });
  // on the pinned Plane, no more of the throttled sign-in calls than it lets through in a minute (support/throttle.ts)
  await paceThePin(context);
  const page = await context.newPage();
  page.setDefaultTimeout(ACTION_TIMEOUT_MS);
  return { context, page };
}
