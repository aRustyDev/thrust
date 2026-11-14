// Journey M2-password (issue #46, the locked-out person's last step): the stock app's Profile -> Security page changes the account's
// password, and the server then does what a person who changed it expects: the browser that changed it stays signed in, a second
// browser that was signed in with the old password is signed out, the old password no longer signs in, and the new one does.
//
// Why this is a journey and not a Rust test. The route (`POST /auth/change-password/`, X09, D-039) has a parity fixture, a model and
// contract tests on four targets. What only a browser can say is that THE PAGE works with what the route answers: the form's own
// validation lets the new password through, the CSRF token it fetches is accepted, the cookies that the answer sets are the ones the
// app keeps using afterwards (a `__Host-` cookie with its attributes, in a real Chromium), the app sends the browser whose session
// ended to the sign-in page, and a sign-in form shows the refusal of the old password.
//
// What would make this pass without proving anything, and what stops it:
//   - a toast that says "Password changed successfully." although the server changed nothing: the journey does not stop at the toast.
//     It asks the server's state three ways (the second browser's next page, the old password at the sign-in form, the new one), and
//     the `change-dropped` control (support/controls.ts) answers the change in the browser without sending it: the journey must fail
//     at "the other browser is signed out";
//   - a change that was refused, and a journey that does not notice: the `change-refused` control answers it with the pin's 5135, and
//     the journey must fail at "change the password";
//   - a second browser that never was signed in (so that "signed out" is no news): it is signed in at the sign-in form, and must be
//     on a page of the workspace before the change is made;
//   - a page that did not render: every field is waited for before it is filled, and the sign-in page's email field before a refusal
//     of the old password is believed;
//   - a run on the wrong target, a journey that skips or a page that is right on a fast machine only: the target guard, the count
//     guard of scripts/e2e.sh, and the `slow-page` control, which runs this journey too.
//
// It runs on both targets: the pin does the same, with a 302 to the app and its own cookie where Thrust answers JSON (D-022), and
// the journey asserts only what the person sees.
//
// What is NOT asserted: the strength rule beyond one password it accepts (D-023 is the Rust suites'), the wording of the refusal (the
// two targets word the sign-in failure alike, and the journey looks for the words of the app's own message, nothing more), and that a
// weak new password is refused (the form does not let a weak one be sent: the button stays disabled).
import { expect, test } from "@playwright/test";
import { control, installControl, STEP_CHANGE, STEP_OTHER } from "../support/controls";
import { openJourney } from "../support/context";
import { ACTION_TIMEOUT_MS, paceThePin } from "../support/throttle";
import { judgeLog, watchPending, watchResponses, type AllowedFailure } from "../support/network";
import { assertTarget, wantedTarget } from "../support/target";

// None: the pin answers each route this journey calls, and Thrust does as well. The pages here call routes that the first journey
// does not (the profile settings), and a route that either target answered 404 would be listed here with the reason.
const ALLOWED_FAILURES: AllowedFailure[] = [];

test("the stock app: change the password in Profile, Security; the other browser is signed out, the old password is refused and the new one signs in", async ({
  browser,
  baseURL,
}) => {
  if (!baseURL) throw new Error("no baseURL: scripts/e2e.sh sets THRUST_BASE_URL");

  const stamp = Date.now().toString(36);
  const email = `change-${stamp}@example.test`;
  const workspaceSlug = `change-${stamp}`;
  const oldPassword = "Journey-Pass-Pa55word-Zebra-81";
  const newPassword = "Another-Sample-Pa55word-Yak-27";

  await assertTarget(baseURL, wantedTarget());

  // browser A, whose network log is the journey's (and is the one that is kept, as a HAR)
  const { context, page } = await openJourney(browser, baseURL, "m2-change-password");
  const seen = watchResponses(page, baseURL);
  const pending = watchPending(page);
  const controlled = await installControl(page);
  // browser B, another device: its own context, so its cookies are its own, and no HAR (scripts/e2e.sh holds the HARs to one a journey)
  const otherContext = await browser.newContext({ baseURL, serviceWorkers: "block", viewport: { width: 1400, height: 950 } });
  await paceThePin(otherContext);
  const other = await otherContext.newPage();
  other.setDefaultTimeout(ACTION_TIMEOUT_MS);
  const seenByOther = watchResponses(other, baseURL);
  await installControl(other);

  // sign in at the sign-in form, as a person does: the email, then the password step
  const signIn = async (who: typeof page, password: string) => {
    await who.goto("/");
    await expect(who.locator("input[name=email]")).toBeVisible();
    await who.fill("input[name=email]", email);
    await who.getByRole("button", { name: "Continue" }).click();
    await expect(who.locator("input[type=password]")).toHaveCount(1);
    await who.locator("input[type=password]").fill(password);
    await who.locator("button[type=submit]").click();
  };

  // Where a page is, by its path alone: the sign-in page of a visitor who was sent there from the workspace carries the workspace's path in
  // its query (`/?next_path=/change-x/`), and a match on the whole address would take that for being in the workspace
  const inWorkspace = (who: typeof page) => expect.poll(() => new URL(who.url()).pathname).toMatch(new RegExp(`^/${workspaceSlug}/`));
  const atSignIn = (who: typeof page) => expect.poll(() => new URL(who.url()).pathname).toBe("/");

  let walked: unknown;
  try {
    await test.step("browser A: sign up, onboard, make a workspace", async () => {
      await page.goto("/");
      await page.fill("input[name=email]", email);
      await page.getByRole("button", { name: "Continue" }).click();
      await expect(page.locator("input[type=password]").first()).toBeVisible();
      for (const input of await page.locator("input[type=password]").all()) await input.fill(oldPassword);
      await page.locator("button[type=submit]").click();
      await expect(page).toHaveURL(/\/onboarding\//);
      await page.fill("input[name=first_name]", "Change User");
      await page.getByRole("button", { name: "Continue" }).click();
      await page.fill("input[name=name]", "Change Workspace");
      await page.fill("input[name=slug]", workspaceSlug);
      await page.getByRole("button", { name: "Just myself" }).click();
      await page.getByRole("button", { name: "Create workspace" }).click();
      await inWorkspace(page);
      // what the page asked for on its way in is answered before the next navigation: a `goto` made while the app is still redirecting
      // itself is aborted by the browser (G-31 and the third flake recorded in `thrust-45-e2e-journeys`)
      await pending.quiet();
    });

    await test.step("browser B: the same account, signed in on another device", async () => {
      await signIn(other, oldPassword);
      await inWorkspace(other);
    });

    await test.step("Profile, Security: the form is there and asks for the current password", async () => {
      await page.goto("/settings/profile/security");
      await expect(page.locator("#old_password")).toBeVisible();
      await expect(page.locator("#new_password")).toBeVisible();
      await expect(page.locator("#confirm_password")).toBeVisible();
    });

    await test.step(STEP_CHANGE, async () => {
      await page.fill("#old_password", oldPassword);
      await page.fill("#new_password", newPassword);
      await page.fill("#confirm_password", newPassword);
      await page.getByRole("button", { name: "Change password" }).click();
      await expect(page.getByText("Password changed successfully."), `${STEP_CHANGE}: the app did not say the password was changed`).toBeVisible();
      await pending.quiet();
    });

    await test.step("browser A is still signed in, on the session the change made", async () => {
      // a reload asks the server who is there again, with the cookies the answer to the change set
      await page.reload();
      await expect(page.locator("#old_password")).toBeVisible();
    });

    await test.step(STEP_OTHER, async () => {
      // B's session ended with the change: its next page is the sign-in page, not the workspace
      await other.reload();
      await expect(other.locator("input[name=email]"), `${STEP_OTHER}: the other browser is still signed in after the password was changed`).toBeVisible();
      await atSignIn(other);
    });

    await test.step("the old password is refused at the sign-in form", async () => {
      await signIn(other, oldPassword);
      await expect(other.getByText(/authentication failed/i).first(), "the sign-in form did not say that the old password failed").toBeVisible();
      await atSignIn(other);
    });

    await test.step("the new password signs in", async () => {
      await signIn(other, newPassword);
      await inWorkspace(other);
    });
  } catch (error) {
    walked = error;
  }

  await otherContext.close();
  await context.close(); // writes the HAR

  // --- the network log (both browsers') ----------------------------------------------------------------------------------------
  const log = [...seen, ...seenByOther];
  judgeLog(log, walked, ALLOWED_FAILURES, { assets: 20, api: 20 });

  // the change reached the server, and what the server said is what the page believed (a control that answers it in the browser leaves
  // a 200 in this log too, which is why the journey looks at the server's state and not at this alone)
  expect(
    log.some((s) => s.method === "POST" && s.path === "/auth/change-password/" && s.status === 200),
    "no 200 for POST /auth/change-password/",
  ).toBe(true);

  // a control that did nothing is no control
  if (control === "change-refused") expect(controlled.refused(), "CONTROL INERT: change-refused answered no change of password").toBeGreaterThan(0);
  if (control === "change-dropped") expect(controlled.dropped(), "CONTROL INERT: change-dropped answered no change of password").toBeGreaterThan(0);
});
