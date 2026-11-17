// Journey M2-code (issue #46, F-A04, D-042): the stock app signs a person in with a code that is mailed to them, and nothing else is typed but
// the code. The person has an account (signing up by code is not built, D-042 (1)); they open the sign-in page in a new browser, give their
// email, and at the password step take "Sign in with unique code": the app asks for a code, the person reads it from the mail, gets one wrong,
// asks for another (Resend), finds the first is dead, and signs in with the second.
//
// Why this is a journey and not a Rust test. The routes (`POST /auth/magic-generate/` and `POST /auth/magic-sign-in/`, X12 and X13) have the
// pin's recordings, generated models and four contract cells, and the `mail` pass reads the code from the mail and signs in with it. What only
// a browser can say is that THE PAGE works with what they answer: the app draws the button because `/api/instances/` says mail is on, it asks
// for the code itself and moves to the code step, the hidden `csrfmiddlewaretoken` field of its form and the `__Host-thrust_csrf` cookie it was
// given by another request agree (a form whose token is not the cookie's is a 403 page), the refusal is a 302 that the app turns back into the
// code step with its banner and the address filled in, Resend replaces the code, and the `__Host-` session cookie that the success sets is the
// one the app goes on with (a reload stays in the workspace).
//
// What would make this pass without proving anything, and what stops it (support/controls.ts, scripts/e2e.sh `mail controls`):
//   - a button that is not on the page, or a code step reached some other way: `smtp-off` turns mail off in the browser, and the journey must
//     fail at "ask for a code";
//   - a reader of the mail that hands over any code: the sign-in itself would refuse it, but `mail-unread` asks it for the mail of an address
//     that was never mailed, and the journey must fail at "read the code from the mail";
//   - a journey that stops at "the form was posted" or at the page having a code step: `code-dropped` changes the code in the form to one
//     that no code is, and the journey must fail at "the code signs in";
//   - a code that is accepted twice, or a Resend that does not replace the first code: the first code is posted AFTER the second was mailed,
//     and must be refused;
//   - a run on the wrong target, a journey that skips, or a page that is right on a fast machine only: the target guard, the count guard of
//     scripts/e2e.sh, and the `slow-page` control, which runs this journey too (and slows the page's own request for its CSRF token).
//
// It runs on Thrust only: the pinned Plane keeps its code in its Valkey and its stack has no mail sink (G-44 says so).
//
// What is NOT asserted: the text of the mail (the contract suite's `mail` pass holds it), the limits (four codes a window, five tries a code:
// the Rust suites' models), and the cap of a window being reached: this journey asks for two codes and makes two wrong tries.
import { expect, test, type Page } from "@playwright/test";
import { signUpAndMakeWorkspace } from "../support/account";
import { control, installControl, MAIL_WAIT_MS, STEP_ASK, STEP_READ, STEP_SIGN_IN } from "../support/controls";
import { openJourney } from "../support/context";
import { codeIn, waitForMail } from "../support/mail";
import { judgeLog, watchPending, watchResponses, type AllowedFailure } from "../support/network";
import { assertMailTarget, assertTarget, wantedTarget } from "../support/target";

// None: every route this journey calls answers on the target, and a 302 is no failure.
const ALLOWED_FAILURES: AllowedFailure[] = [];

test("the stock app: ask for a unique code, a wrong one and the replaced one are refused, the mailed one signs in", async ({ browser, baseURL }) => {
  // the control that the count guard of scripts/e2e.sh must catch: Playwright reports a skipped test as a pass
  test.skip(control === "skip", "E2E_CONTROL=skip");
  if (!baseURL) throw new Error("no baseURL: scripts/e2e.sh sets THRUST_BASE_URL");

  const stamp = Date.now().toString(36);
  const email = `code-${stamp}@example.test`;
  const workspaceSlug = `code-${stamp}`;
  const password = "Journey-Pass-Pa55word-Zebra-81";

  const target = wantedTarget();
  await assertTarget(baseURL, target);
  await assertMailTarget(baseURL);

  const { context, page } = await openJourney(browser, baseURL, "m2-sign-in-by-code");
  const seen = watchResponses(page, baseURL);
  const pending = watchPending(page);
  const controlled = await installControl(page);

  // the mail of the address the journey signed up with; the `mail-unread` control looks for the mail of one that was never mailed
  const mailbox = control === "mail-unread" ? `nobody-${email}` : email;
  // the code of the n-th mail to the address, or the failure that says which step could not read it
  const codeOfMail = async (n: number): Promise<string> => {
    try {
      const mails = await waitForMail(mailbox, n, MAIL_WAIT_MS);
      const mail = mails[n - 1];
      if (!mail) throw new Error(`no mail number ${n}`);
      return codeIn(mail);
    } catch (error) {
      throw new Error(`${STEP_READ}: ${error instanceof Error ? error.message : String(error)}`);
    }
  };

  // post the code form and give the server's answer to it: the form is a native post, answered by a 302, and the page that follows is the app's
  const submitCode = async (who: Page, code: string) => {
    const posted = who.waitForResponse((r) => r.request().method() === "POST" && new URL(r.url()).pathname === "/auth/magic-sign-in/");
    await who.fill("#unique-code", code);
    await who.getByRole("button", { name: "Continue" }).click();
    return await posted;
  };
  const location = (response: { headers: () => Record<string, string> }) => response.headers()["location"] ?? "";

  // where the page is, by its path alone (the sign-in page of a visitor sent there from the workspace carries the workspace's path in its query)
  const inWorkspace = (message: string) =>
    expect.poll(() => new URL(page.url()).pathname, { message }).toMatch(new RegExp(`^/${workspaceSlug}/`));

  let walked: unknown;
  try {
    await test.step("sign up, onboard, make a workspace: the account that has a password and will be signed in by a code", async () => {
      await signUpAndMakeWorkspace(page, { email, password, name: "Code User", workspaceSlug }, () => pending.quiet());
    });

    await test.step("a new visit: the sign-in page, the email, the password step", async () => {
      // a new visitor: no cookies, so the app asks again who is there
      await context.clearCookies();
      await page.goto("/");
      await expect(page.locator("input[name=email]")).toBeVisible();
      await page.fill("input[name=email]", email);
      await page.getByRole("button", { name: "Continue" }).click();
      await expect(page.locator("input[type=password]")).toHaveCount(1);
    });

    await test.step(STEP_ASK, async () => {
      // the page draws the button because the instance says mail is on, and the click is what asks for the code
      const unique = page.getByRole("button", { name: "Sign in with unique code" });
      await expect(unique, `${STEP_ASK}: the app does not draw "Sign in with unique code" on the password step`).toBeVisible();
      const asked = page.waitForResponse((r) => r.request().method() === "POST" && new URL(r.url()).pathname === "/auth/magic-generate/");
      await unique.click();
      expect((await asked).status(), `${STEP_ASK}: POST /auth/magic-generate/ was not answered 200`).toBe(200);
      await expect(page.locator("#unique-code"), `${STEP_ASK}: the app did not move to the code step`).toBeVisible();
      await expect(page.locator("#email"), `${STEP_ASK}: the code step does not show the address`).toHaveValue(email);
    });

    // the first mail, its code; and the page's own requests settled, so that the form's hidden token field has what its cookie has (the page
    // fetched a token for the password form and another for this one; a person takes seconds to read a mail, and so does this)
    const first = await codeOfMail(1);
    await pending.quiet();
    await expect(page.locator("input[name=csrfmiddlewaretoken]"), "the code form's hidden CSRF field is empty").not.toHaveValue("");

    await test.step("a wrong code is refused, and the app shows the code step again with its banner", async () => {
      // 000000 is no code: they are 100000 to 999999
      const refusal = await submitCode(page, "000000");
      expect(refusal.status(), "the form's post was not answered by a redirect").toBe(302);
      expect(location(refusal), "a wrong code was not refused with the pin's INVALID_MAGIC_CODE_SIGN_IN (5090)").toContain("error_code=5090");
      await expect(page.getByText("Invalid magic code. Please try again.")).toBeVisible();
      await expect(page.locator("#unique-code")).toBeVisible();
      await expect(page.locator("#email")).toHaveValue(email);
      await pending.quiet();
    });

    await test.step("Resend mails a second code, and the first code is dead", async () => {
      await page.getByRole("button", { name: "Resend" }).click();
      const second = await codeOfMail(2);
      expect(second, "Resend mailed the same code again").not.toBe(first);
      await pending.quiet();
      // the first code was right until the second was made: it is refused now
      const stale = await submitCode(page, first);
      expect(stale.status()).toBe(302);
      expect(location(stale), "the first code still signs in after Resend replaced it").toContain("error_code=5090");
      await expect(page.locator("#unique-code")).toBeVisible();
      await pending.quiet();
    });

    await test.step(STEP_SIGN_IN, async () => {
      const second = await codeOfMail(2);
      const signedIn = await submitCode(page, second);
      expect(signedIn.status(), `${STEP_SIGN_IN}: the form's post was not answered by a redirect`).toBe(302);
      expect(location(signedIn), `${STEP_SIGN_IN}: the server refused the mailed code (${location(signedIn)})`).not.toContain("error_code");
      await inWorkspace(`${STEP_SIGN_IN}: the person is not in their workspace after the code`);
      await pending.quiet();
    });

    await test.step("the session the code made is the one the app goes on with: a reload stays in the workspace", async () => {
      await page.reload();
      await inWorkspace("a reload after the code's sign-in left the workspace");
      const session = (await context.cookies()).find((c) => c.name === "__Host-thrust_session");
      expect(session, "the sign-in by code set no __Host-thrust_session cookie").toBeDefined();
      expect(session?.httpOnly, "the session cookie is readable by the page's scripts").toBe(true);
      expect(session?.secure, "the session cookie is not Secure").toBe(true);
    });
  } catch (error) {
    walked = error;
  }

  await context.close(); // writes the HAR

  // --- the network log ---------------------------------------------------------------------------------------------------------
  judgeLog(seen, walked, ALLOWED_FAILURES, { assets: 20, api: 20 });

  // two codes were asked for (the button, and Resend), and the form was posted three times, each answered by a redirect
  const statusesOf = (path: string) => seen.filter((s) => s.method === "POST" && s.path === path).map((s) => s.status);
  expect(statusesOf("/auth/magic-generate/"), "POST /auth/magic-generate/").toEqual([200, 200]);
  expect(statusesOf("/auth/magic-sign-in/"), "POST /auth/magic-sign-in/").toEqual([302, 302, 302]);

  // a control that did nothing is no control
  if (control === "smtp-off") expect(controlled.mailTurnedOff(), "CONTROL INERT: smtp-off rewrote no /api/instances/ answer").toBeGreaterThan(0);
  if (control === "code-dropped") expect(controlled.codesChanged(), "CONTROL INERT: code-dropped changed no code").toBeGreaterThan(0);
});
