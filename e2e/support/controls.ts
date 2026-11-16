// The controls: ways of running a journey that MUST make it fail, so that a pass means something (AGENTS.md testing rule 8). Each is
// chosen by E2E_CONTROL, and scripts/e2e.sh (`controls`) runs it and requires the failure AND the words it fails with.
//
//   stickies-404  one route the journey calls answers 404 in the browser, as if it were not mounted. The journey must fail naming
//                 that 404: a detector that cannot see a 404 would pass every run.
//   empty-list    the browser empties the work-item list (S47) that the server answers, keeping the envelope. The journey must fail at
//                 "see it in the list": a journey that never looks at the server's list would pass with this.
//   skip          the journey skips itself, which Playwright reports as a pass (exit 0). scripts/e2e.sh's count guard must catch it.
//   sso-on        the browser turns two sign-in providers on in the answer of `/api/instances/` (the flags the app draws its buttons from).
//                 The sign-in journey must fail, saying "a sign-in method other than the password": a journey that cannot see a button
//                 that Thrust has no route for would pass with it.
//   slow-page     the odd one out: this run MUST PASS. The browser is slowed to a CI runner's pace, and the profile fetch that the
//                 workspace's home page makes as it mounts is slow as well. G-31 is the failure it stands for: the app marks the welcome
//                 tour as done in its store at the click, and keeps what a profile fetch returns after that, so a profile answer made
//                 before the click and landing after it puts the tour back for good, although the PATCH was answered 200. On a fast
//                 machine the page does not make that fetch at all, which is why the slowness is part of the control. The held answer
//                 is the server's own, released when the tour's PATCH leaves the browser. A journey that answers the tour once and goes
//                 on fails this run; one that looks that the tour stayed answered, and answers it again if it did not, does not.
//                 A journey that has no workspace (the sign-in journey) is slowed and holds nothing.
//
//   change-refused  the browser answers the change of password (`POST /auth/change-password/`) with the pin's 5135, "Old password is not correct",
//                 and sends it nowhere. The password journey must fail at "change the password": a journey that does not look at what the app
//                 says about the change would go on to check a server that was never asked.
//   change-dropped  the browser answers the change with the pin's 200 and sends it nowhere, so that the app says "Password changed
//                 successfully." and the server has changed nothing. The password journey must fail at "the other browser is signed out":
//                 a journey that stops at the toast, or at the 200 in its network log (which this leaves), would pass with it.
//
// The mail suite's (e2e/journeys-mail/, scripts/e2e.sh `mail`), each one a run of its journey that must fail:
//
//   smtp-off      the browser says in the answer of `/api/instances/` that no mail service is configured, so that the app does not draw
//                 "Sign in with unique code". The journey must fail at "ask for a code": a journey that does not look for the button
//                 (or that reaches the code step some other way) would pass with it.
//   code-dropped  the browser changes the code in the form it posts to `/auth/magic-sign-in/` to 000000, which no code is (they are 100000 to
//                 999999), and sends it: the real server refuses it, and the app is sent back to the code step with its banner. The journey
//                 must fail at "the code signs in": a journey that stops at "the form was posted", or at the page showing the code step,
//                 would pass with it.
//   mail-unread   the journey asks the simulator's log for the mail of an address that was never mailed, and waits 3 s for it. It must fail
//                 at "read the code from the mail": a reader that finds a message whatever it is asked for would hand the journey a code
//                 that is not its own (the server would then refuse it, which is why the sign-in is the journey's real check; this control
//                 is the reader's own).
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
// the words the sign-in journey fails with when a method other than the password is on screen; scripts/e2e.sh's `sso-on` control greps for them
export const STEP_METHODS = "a sign-in method other than the password";
// the password journey's two steps that the `change-refused` and `change-dropped` controls must fail at, by these words
export const STEP_CHANGE = "change the password";
export const STEP_OTHER = "the other browser is signed out";
// the code journey's three steps that `smtp-off`, `mail-unread` and `code-dropped` must fail at, by these words
export const STEP_ASK = "ask for a code";
export const STEP_READ = "read the code from the mail";
export const STEP_SIGN_IN = "the code signs in";
// how long the code journey waits for a mail, and for the one the `mail-unread` control asks for, which never comes
export const MAIL_WAIT_MS = control === "mail-unread" ? 3_000 : 20_000;

// how many times slower than this machine the browser runs under `slow-page` (E2E_CPU_RATE overrides it)
const CPU_RATE = Number(process.env.E2E_CPU_RATE ?? "6");
// the user's profile (GET), and how long at most the answer to the fetch from inside the workspace is held: more than a journey takes
// to click what is on screen, little enough to be cheap
const PROFILE = /\/api\/users\/me\/profile\/$/;
const LATE_MS = 2_000;

// `where` names the workspace the journey made, for the control that looks at what happens inside it; a journey with none passes nothing
export async function installControl(
  page: Page,
  where: { workspaceSlug?: string } = {},
): Promise<{
  emptied: () => number;
  delayed: () => number;
  providersTurnedOn: () => number;
  refused: () => number;
  dropped: () => number;
  mailTurnedOff: () => number;
  codesChanged: () => number;
}> {
  let emptied = 0;
  let delayed = 0;
  let providersTurnedOn = 0;
  let refused = 0;
  let dropped = 0;
  let mailTurnedOff = 0;
  let codesChanged = 0;
  let taken = false;

  if (control === "smtp-off") {
    await page.route("**/api/instances/", async (route) => {
      const response = await route.fetch();
      const body: unknown = await response.json().catch(() => undefined);
      const config = typeof body === "object" && body !== null ? (body as { config?: unknown }).config : undefined;
      if (response.status() !== 200 || typeof config !== "object" || config === null) return route.fulfill({ response });
      mailTurnedOff += 1;
      return route.fulfill({
        response,
        json: { ...(body as object), config: { ...config, is_smtp_configured: false, is_magic_login_enabled: false } },
      });
    });
  }

  if (control === "code-dropped") {
    // the form is a native post (urlencoded: `email`, `code`, `csrfmiddlewaretoken`), so the page is not told: its next page is the server's answer
    await page.route("**/auth/magic-sign-in/", async (route) => {
      const request = route.request();
      if (request.method() !== "POST") return route.fallback();
      const form = new URLSearchParams(request.postData() ?? "");
      if (!form.has("code")) return route.fallback();
      form.set("code", "000000");
      codesChanged += 1;
      return route.continue({ postData: form.toString() });
    });
  }

  if (control === "sso-on") {
    await page.route("**/api/instances/", async (route) => {
      const response = await route.fetch();
      const body: unknown = await response.json().catch(() => undefined);
      const config = typeof body === "object" && body !== null ? (body as { config?: unknown }).config : undefined;
      if (response.status() !== 200 || typeof config !== "object" || config === null) return route.fulfill({ response });
      providersTurnedOn += 1;
      return route.fulfill({
        response,
        json: { ...(body as object), config: { ...config, is_oidc_enabled: true, is_google_enabled: true } },
      });
    });
  }

  if (control === "change-refused" || control === "change-dropped") {
    // the change of password, which the app posts from Profile -> Security: answered here and never sent to the server
    await page.route("**/auth/change-password/", async (route) => {
      if (route.request().method() !== "POST") return route.fallback();
      if (control === "change-refused") {
        refused += 1;
        return route.fulfill({
          status: 400,
          contentType: "application/json",
          body: JSON.stringify({ error_code: 5135, error_message: "INCORRECT_OLD_PASSWORD", error: "Old password is not correct" }),
        });
      }
      dropped += 1;
      return route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ message: "Password updated successfully" }) });
    });
  }

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
    // tour is answered, and then released: so, made before the click, it lands in the moment between that click and the next. LATE_MS
    // is the longest it is held, for a journey that waits for it before it clicks
    const answered = new Promise<void>((resolve) => {
      page.on("request", (r) => {
        if (r.method() === "PATCH" && /\/api\/users\/me\/tour-completed\/$/.test(new URL(r.url()).pathname)) resolve();
      });
    });
    await page.route(PROFILE, async (route) => {
      const inWorkspace =
        where.workspaceSlug !== undefined && new URL(route.request().frame().url()).pathname.startsWith(`/${where.workspaceSlug}/`);
      if (route.request().method() !== "GET" || taken || !inWorkspace) return route.fallback();
      taken = true;
      const response = await route.fetch();
      const body: unknown = await response.json().catch(() => undefined);
      await Promise.race([answered, new Promise((resolve) => setTimeout(resolve, LATE_MS))]);
      if (response.status() !== 200 || typeof body !== "object" || body === null || !("is_tour_completed" in body)) {
        return route.fulfill({ response });
      }
      delayed += 1;
      // as the server gave it when it was asked, which is before the tour was answered: a fetch made after the click is answered
      // `true`, as it would be, and is no stale answer
      return route.fulfill({ response });
    });
  }

  return {
    emptied: () => emptied,
    delayed: () => delayed,
    providersTurnedOn: () => providersTurnedOn,
    refused: () => refused,
    dropped: () => dropped,
    mailTurnedOff: () => mailTurnedOff,
    codesChanged: () => codesChanged,
  };
}
