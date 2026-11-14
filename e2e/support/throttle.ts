// The pinned Plane throttles the three calls a sign-in or a sign-up makes (`/auth/email-check/`, `/auth/sign-in/`, `/auth/sign-up/`) to
// 10 a minute per address (`AuthenticationThrottle`, `AUTHENTICATION_RATE_LIMIT`, apps/api/plane/authentication/rate_limit.py; DRF counts
// the refused calls too), and answers the 11th with a 429. The journeys are one address to it, so what each of them calls before the
// next one starts is part of what the next one gets: the password journey (a second browser, a refused sign-in, a sign-in) makes 8 of
// them, and the sign-in journey after it was answered 429 on its first email check (the first run on the pin).
//
// So on the pin the browser asks for no more than the pin lets through: this holds a call of the three until the last minute has room for
// it, the way tools/compat-record/record.py's `Pacer` does for its own journey. Every context of every journey goes through the same
// window (one worker process runs them all, so the module's state is shared). Thrust has no per-address throttle on these routes
// (it limits sign-in per email, in the core, and a journey that goes over it is a finding), so there nothing is held.
//
// The held call is held in the browser's request, and the page waits for its answer as it would for a slow server: which is why the
// expect timeout is longer on the pin (playwright.config.ts).
import type { BrowserContext } from "@playwright/test";
import { wantedTarget } from "./target";

const LIMIT = 10;
const WINDOW_MS = 60_000;
// the pin's window is its own clock's, not this one's: a margin for the difference and for the network, as the recorder keeps
const MARGIN_MS = 1_500;
const THROTTLED = /\/auth\/(email-check|sign-in|sign-up)\//;

const calls: number[] = [];

// how long a page action may take: a click that submits the sign-in form waits for the navigation it starts, and on the pin that
// request can be held for most of a minute
export const ACTION_TIMEOUT_MS = wantedTarget() === "plane" ? 90_000 : 30_000;

// resolves when a call may be made, and counts it
export async function roomForACall(): Promise<void> {
  for (;;) {
    const now = Date.now();
    while (calls.length > 0 && now - calls[0] >= WINDOW_MS + MARGIN_MS) calls.shift();
    if (calls.length < LIMIT) {
      calls.push(now);
      return;
    }
    await new Promise((resolve) => setTimeout(resolve, calls[0] + WINDOW_MS + MARGIN_MS - now));
  }
}

// every context a journey opens goes through this: on the pin it holds the three throttled calls to the window, elsewhere it does nothing
export async function paceThePin(context: BrowserContext): Promise<void> {
  if (wantedTarget() !== "plane") return;
  await context.route(THROTTLED, async (route) => {
    // the page's own navigation to a path that is not a call (the sign-in page is `/`) is not held: only a POST is a call
    if (route.request().method() !== "POST") return route.fallback();
    await roomForACall();
    return route.fallback();
  });
}
