// Journey M2-auth (the locked-out person, issue #46's sign-in screen): the stock app's sign-in page offers the password and nothing else,
// and the one control that stands where a reset would be sends no request to a route that does not exist.
//
// Why this is a journey and not a Rust test. The buttons are drawn by the app from what `/api/instances/` says (`is_oidc_enabled`,
// `is_magic_login_enabled`, `is_smtp_configured`, ...). Thrust answers all of them off, as the pin does (S01), so the social buttons and the
// code step never render. That the answer is right is the parity fixture's; that the SCREEN then shows no way in that Thrust cannot serve
// is only a browser's to say, and it is what would break first if someone turned a flag on before building its route (password reset,
// email code and social login have no route; PR #130, D-037).
//
// What would make this pass without proving anything, and what stops it:
//   - a page that did not render: the email field must be on screen before anything is counted absent, and the sign-in step is reached
//     through a real account (an existing email is what makes the app show a sign-in and not a sign-up);
//   - a detector that cannot see a method on screen: the `sso-on` control (support/controls.ts) turns two providers on in the browser and
//     the journey must fail, saying "a sign-in method other than the password";
//   - a journey that skips itself: the count guard of scripts/e2e.sh.
//
// It runs on both targets. On the pin the same page shows the popover that says SMTP is off; on Thrust (which changed `password.tsx`,
// Q-R2) it shows the line that says whom to ask. Both are asserted, each on its own target.
import { expect, test } from "@playwright/test";
import { control, installControl, STEP_METHODS } from "../support/controls";
import { openJourney } from "../support/context";
import { judgeLog, watchResponses, type AllowedFailure } from "../support/network";
import { assertTarget, wantedTarget } from "../support/target";

const ALLOWED_FAILURES: AllowedFailure[] = [];

// routes of a sign-in method that Thrust has not built (and that the pin only calls when its SMTP or a provider is configured)
const METHOD_ROUTES = /^\/auth\/(forgot-password|reset-password|magic-generate|magic-sign-in|set-password|oidc|google|github|gitlab|gitea)\b/;

test("the stock app: the sign-in page offers the password and nothing else, and the forgot-password control sends no request", async ({
  browser,
  baseURL,
}) => {
  if (!baseURL) throw new Error("no baseURL: scripts/e2e.sh sets THRUST_BASE_URL");

  const target = wantedTarget();
  const stamp = Date.now().toString(36);
  const email = `signin-${stamp}@example.test`;
  const password = "Journey-Pass-Pa55word-Zebra-81";

  await assertTarget(baseURL, target);

  const { context, page } = await openJourney(browser, baseURL, "m2-sign-in-methods");
  const seen = watchResponses(page, baseURL);
  const controlled = await installControl(page);

  // a method of signing in other than the password: a provider's button ("Sign in with Google", "...with SSO") or the unique code
  const otherMethods = page.getByText(/sign in with|sign up with/i);

  let walked: unknown;
  try {
    await test.step("make an account, so that the next visit is a sign-in", async () => {
      await page.goto("/");
      await page.fill("input[name=email]", email);
      await page.getByRole("button", { name: "Continue" }).click();
      await expect(page.locator("input[type=password]").first()).toBeVisible();
      for (const input of await page.locator("input[type=password]").all()) await input.fill(password);
      await page.locator("button[type=submit]").click();
      await expect(page).toHaveURL(/\/onboarding\//);
    });

    await test.step("the sign-in page: the email field, and no method but the password", async () => {
      // a new visitor: no cookies, so the app asks again who is there
      await context.clearCookies();
      await page.goto("/");
      // the page has rendered (an absent button on a blank page proves nothing)
      await expect(page.locator("input[name=email]")).toBeVisible();
      await expect(page.getByText("No authentication methods available")).toHaveCount(0);
      expect(await otherMethods.count(), `${STEP_METHODS} is on the sign-in page`).toBe(0);
    });

    await test.step("an existing email goes to the password step, where the forgot-password control is", async () => {
      await page.fill("input[name=email]", email);
      await page.getByRole("button", { name: "Continue" }).click();
      await expect(page.locator("input[type=password]")).toHaveCount(1);
      // still no unique-code button ("Sign in with unique code" is shown only when the instance says SMTP is on)
      expect(await otherMethods.count(), `${STEP_METHODS} is on the password step`).toBe(0);
      await expect(page.getByText("Forgot your password?").first()).toBeVisible();

      if (target === "thrust") {
        // Thrust (web/ALLOWED-CHANGES, password.tsx): a line that says whom to ask, and no control that does something
        const hint = page.getByTestId("forgot-password-operator-hint");
        await expect(hint).toBeVisible();
        await expect(hint).toContainText("Ask your admin");
      } else {
        // the pin: a popover that says SMTP is off
        await page.getByRole("button", { name: "Forgot your password?" }).click();
        await expect(page.getByText(/SMTP/)).toBeVisible();
      }
    });

    await test.step("the password still signs in", async () => {
      await page.locator("input[type=password]").fill(password);
      await page.locator("button[type=submit]").click();
      await expect(page).toHaveURL(/\/onboarding\//);
    });
  } catch (error) {
    walked = error;
  }

  await context.close(); // writes the HAR

  // the page asked for no route of a method that has none, on either step
  const methodCalls = seen.filter((s) => METHOD_ROUTES.test(s.path)).map((s) => `${s.method} ${s.path} -> ${s.status}`);

  judgeLog(seen, walked, ALLOWED_FAILURES, { assets: 20, api: 3 });
  expect(methodCalls, "the sign-in page called a route of a sign-in method that is not built").toEqual([]);

  // a control that did nothing is no control
  if (control === "sso-on") expect(controlled.providersTurnedOn(), "CONTROL INERT: sso-on rewrote no /api/instances/ answer").toBeGreaterThan(0);
});
